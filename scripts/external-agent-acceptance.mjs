#!/usr/bin/env node
// Opt-in acceptance run for external agents (Claude Code, Codex, OpenCode)
// against a running Mira server: the packaged desktop app's server, or
// `mira serve`. It uses real, signed-in CLIs and spends real turns, so it
// only runs with MIRA_ACCEPTANCE=1.
//
//   MIRA_ACCEPTANCE=1 node scripts/external-agent-acceptance.mjs
//
//   MIRA_URL               server to drive (default http://127.0.0.1:8787)
//   MIRA_ACCEPTANCE_AGENTS engine instances, comma-separated
//                          (default claude-code,codex,opencode)
//   MIRA_ACCEPTANCE_MODEL_<INSTANCE>  model to start with, e.g.
//                          MIRA_ACCEPTANCE_MODEL_OPENCODE=opencode/big-pickle
//   MIRA_ACCEPTANCE_SWITCH_MODEL_<INSTANCE>  model for the model-change
//                          scenario (default: another model on the same provider)
//   MIRA_ACCEPTANCE_OUT    report directory (default ./acceptance-report)
//
// Each agent gets its own throwaway git project. Results are `pass`, `fail`
// (with reproduction steps and sanitized evidence), `unavailable` (the
// installed agent does not offer the capability; not a failure), `blocked`
// (a precondition such as provider sign-in is missing), or
// `manual` (needs a person in the UI; see docs/external-agent-acceptance.md).
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

if (process.env.MIRA_ACCEPTANCE !== '1') {
  console.error('Refusing to run: set MIRA_ACCEPTANCE=1. This uses signed-in agent CLIs and spends real turns.');
  process.exit(2);
}
const BASE = (process.env.MIRA_URL ?? 'http://127.0.0.1:8787').replace(/\/$/, '');
const AGENTS = (process.env.MIRA_ACCEPTANCE_AGENTS ?? 'claude-code,codex,opencode').split(',').map((s) => s.trim()).filter(Boolean);
const OUT = process.env.MIRA_ACCEPTANCE_OUT ?? 'acceptance-report';
const TURN_MS = 180_000;
const AUTH = /authentication required|not (logged|signed) in|unauthori[sz]ed|login required/i;

/** Long values and anything that looks like a credential are cut from evidence. */
function sanitize(value) {
  const text = typeof value === 'string' ? value : JSON.stringify(value);
  return text
    .replace(/(sk-[A-Za-z0-9_-]{8})[A-Za-z0-9_-]+/g, '$1…')
    .replace(/("(?:token|api_key|authorization|secret)[^"]*"\s*:\s*")[^"]+/gi, '$1«redacted»')
    .slice(0, 600);
}

async function http(method, url, body, session) {
  const headers = { 'content-type': 'application/json' };
  if (session) headers['X-Mira-Session'] = session;
  const r = await fetch(BASE + url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  if (!r.ok) throw new Error(`${method} ${url} → ${r.status}: ${sanitize(json?.error ?? text)}`);
  return json;
}

/** One watched chat: every frame is kept so waits never miss an early one. */
class Chat {
  constructor(id) { this.id = id; this.frames = []; this.waiters = []; }
  connect() {
    const url = `${BASE.replace(/^http/, 'ws')}/ws?session_id=${encodeURIComponent(this.id)}`;
    return new Promise((resolve, reject) => {
      this.ws = new WebSocket(url);
      this.ws.onmessage = (ev) => {
        if (typeof ev.data !== 'string') return;
        const frame = JSON.parse(ev.data);
        this.frames.push(frame);
        for (const w of [...this.waiters]) if (w.test(frame)) { this.waiters.splice(this.waiters.indexOf(w), 1); w.resolve(frame); }
        if (frame.type === 'ready') resolve(frame);
      };
      this.ws.onerror = () => reject(new Error(`websocket ${url} failed`));
    });
  }
  send(msg) { this.ws.send(JSON.stringify(msg)); }
  /** The next frame (from `since`) matching `test`. */
  waitFor(test, ms = TURN_MS, since = 0) {
    const seen = this.frames.slice(since).find(test);
    if (seen) return Promise.resolve(seen);
    return new Promise((resolve, reject) => {
      const w = { test, resolve };
      this.waiters.push(w);
      setTimeout(() => { this.waiters.splice(this.waiters.indexOf(w), 1); reject(new Error(`timed out after ${ms / 1000}s`)); }, ms);
    });
  }
  mark() { return this.frames.length; }
  textSince(i) {
    return this.frames.slice(i).filter((f) => f.type === 'acp_text' || f.type === 'acp_text_snapshot').map((f) => f.text).join('');
  }
  engine() { return [...this.frames].reverse().find((f) => f.type === 'session_engine')?.engine ?? null; }
  close() { this.ws?.close(); }
}

async function prompt(chat, text) {
  const at = chat.mark();
  chat.send({ type: 'acp_prompt', text });
  const end = await chat.waitFor((f) => f.type === 'acp_turn_end' || f.type === 'acp_failure', TURN_MS, at);
  return { end, text: chat.textSince(at), at };
}

function cliVersion(binary) {
  try {
    return execFileSync(binary, ['--version'], { encoding: 'utf8', timeout: 10_000 }).trim().split('\n')[0];
  } catch { return null; }
}

function project() {
  const dir = mkdtempSync(path.join(tmpdir(), 'mira-acceptance-'));
  writeFileSync(path.join(dir, 'README.md'), '# Acceptance project\n');
  execFileSync('git', ['init', '-q'], { cwd: dir });
  execFileSync('git', ['-c', 'user.name=Mira', '-c', 'user.email=acceptance@localhost', 'commit', '-qam', 'init', '--allow-empty'], { cwd: dir });
  execFileSync('git', ['add', '-A'], { cwd: dir });
  return dir;
}

async function runAgent(instance, engines) {
  const results = [];
  const record = (scenario, status, detail = {}) => {
    results.push({ scenario, status, ...detail });
    console.log(`  ${status.padEnd(11)} ${scenario}${detail.reason ? ` — ${detail.reason}` : ''}`);
  };
  const failed = (scenario, steps, error, chat) => {
    const auth = chat?.frames.slice(-12).find((f) => f.type === 'error' && AUTH.test(f.text ?? ''));
    if (auth) return record(scenario, 'blocked', { reason: `not signed in for this model: ${sanitize(auth.text)}` });
    return record(scenario, 'fail', {
      reason: sanitize(error?.message ?? String(error)),
      steps,
      evidence: chat ? chat.frames.slice(-8).map((f) => sanitize(f)) : [],
    });
  };
  const info = engines.find((e) => (e.instance ?? e.id) === instance);
  const meta = { instance, driver: info?.driver ?? null, state: info?.state ?? null, cli_version: null };
  if (!info) { record('readiness', 'unavailable', { reason: 'no such engine instance on this server' }); return { meta, results }; }
  const binary = { 'claude-code': 'claude', codex: 'codex', opencode: 'opencode' }[info.driver] ?? info.driver;
  meta.cli_version = cliVersion(typeof info.launch === 'string' ? info.launch.split(' ')[0] : binary);
  const ready = (info.state?.state ?? info.state) === 'ready';
  record('readiness', ready ? 'pass' : 'unavailable', ready ? {} : { reason: `engine state ${sanitize(info.state)}` });
  if (!ready) return { meta, results };

  const dir = project();
  const { session_id: id } = await http('PUT', '/api/cwd', { path: dir });
  const chat = new Chat(id);
  await chat.connect();
  const model = process.env[`MIRA_ACCEPTANCE_MODEL_${instance.toUpperCase().replace(/[^A-Z0-9]/g, '_')}`] ?? null;
  chat.send({ type: 'acp_start', instance, model });
  try {
    await chat.waitFor((f) => f.type === 'acp_agent_started', 60_000);
    const started = chat.frames.find((f) => f.type === 'acp_agent_started');
    if (started.error) throw new Error(started.error);
    record('start', 'pass');
  } catch (e) { failed('start', ['Open a new chat in the test project', `Pick ${instance}`], e, chat); return { meta, results }; }

  // A turn, end to end. Missing provider sign-in blocks the rest: those
  // results would describe the account, not Mira.
  try {
    const r = await prompt(chat, 'Reply with exactly the word: pineapple');
    const error = chat.frames.slice(r.at).find((f) => f.type === 'error')?.text ?? '';
    if (AUTH.test(error)) {
      record('prompt', 'blocked', { reason: `the agent is not signed in: ${sanitize(error)}` });
      record('readiness reports sign-in', 'fail', {
        reason: 'the engine reported ready, but its first turn needed provider authentication',
        steps: ['Use an agent home with no provider login', 'Open Settings → External agents', 'Send a prompt'],
        evidence: [sanitize(error)],
      });
      chat.close();
      return { meta, results };
    }
    if (r.end.type === 'acp_failure' || !/pineapple/i.test(r.text)) throw new Error(`stop ${sanitize(r.end)}; text ${sanitize(r.text)}`);
    record('prompt', 'pass');
  } catch (e) { failed('prompt', ['Send "Reply with exactly the word: pineapple"'], e, chat); }
  const engine = chat.engine();
  meta.agent_version = engine?.agent_version ?? null;
  meta.capability_source = engine?.capability_source ?? null;
  meta.capabilities = engine?.capabilities ?? null;

  // Model change on the same native conversation.
  const options = [...chat.frames].reverse().find((f) => f.type === 'acp_config_options')?.options ?? [];
  const modelOption = options.find((o) => o.category === 'model');
  // Another model on the same provider (signed in already), unless named.
  const key = `MIRA_ACCEPTANCE_SWITCH_MODEL_${instance.toUpperCase().replace(/[^A-Z0-9]/g, '_')}`;
  const provider = (v) => (v.includes('/') ? v.split('/')[0] : '');
  const other = process.env[key]
    ? { value: process.env[key] }
    : modelOption?.values?.find((v) => v.value !== modelOption.current && provider(v.value) === provider(modelOption.current ?? ''));
  const original = modelOption?.current;
  if (!other) record('model change', 'unavailable', { reason: 'the agent advertised one model or none' });
  else {
    try {
      const at = chat.mark();
      chat.send({ type: 'acp_set_config_option', option_id: modelOption.id, value: other.value });
      await chat.waitFor((f) => f.type === 'acp_config_options' && f.options.some((o) => o.id === modelOption.id && o.current === other.value), 60_000, at);
      const r = await prompt(chat, 'Reply with exactly the word: mango');
      if (!/mango/i.test(r.text)) throw new Error(`text ${sanitize(r.text)}`);
      record('model change', 'pass', { model: other.value });
    } catch (e) { failed('model change', [`Pick model ${other.value}`, 'Send "Reply with exactly the word: mango"'], e, chat); }
    // Later scenarios run on the model the run started with.
    if (original) {
      const at = chat.mark();
      chat.send({ type: 'acp_set_config_option', option_id: modelOption.id, value: original });
      await chat.waitFor((f) => f.type === 'acp_config_options' && f.options.some((o) => o.id === modelOption.id && o.current === original), 60_000, at).catch(() => {});
    }
  }

  // Cancellation of a long turn.
  try {
    const at = chat.mark();
    chat.send({ type: 'acp_prompt', text: 'Count from 1 to 400, one number per line, with no other text.' });
    await chat.waitFor((f) => f.type === 'acp_text', 90_000, at);
    chat.send({ type: 'interrupt' });
    const end = await chat.waitFor((f) => f.type === 'acp_turn_end', 30_000, at);
    if (!/cancel|interrupt|abort/i.test(end.stop_reason)) throw new Error(`ended with ${end.stop_reason}`);
    record('cancellation', 'pass', { stop_reason: end.stop_reason });
  } catch (e) { failed('cancellation', ['Ask it to count to 400', 'Press Stop once text streams'], e, chat); }

  // Steering during a running turn.
  if (engine?.capabilities?.steering !== 'native') record('steering', 'unavailable', { reason: `steering is ${engine?.capabilities?.steering ?? 'unknown'}` });
  else {
    try {
      const at = chat.mark();
      chat.send({ type: 'acp_prompt', text: 'Count from 1 to 60, one number per line. Then say done.' });
      await chat.waitFor((f) => f.type === 'acp_text', 90_000, at);
      // Steering moves a queued input into the running turn.
      const queued = `acceptance-s-${Date.now()}`;
      const text = 'After counting, also say the word kiwi.';
      chat.send({ type: 'queue_input', session_id: id, id: queued, text });
      await chat.waitFor((f) => f.type === 'queue_updated' && f.items.some((i) => i.id === queued), 30_000, at);
      chat.send({ type: 'steer', request_id: queued, text });
      const steer = await chat.waitFor((f) => f.type === 'steer_result', 60_000, at);
      if (steer.error) throw new Error(steer.error);
      await chat.waitFor((f) => f.type === 'acp_turn_end', TURN_MS, at);
      record('steering', 'pass');
    } catch (e) { failed('steering', ['Ask it to count to 60', 'Steer: "also say kiwi"'], e, chat); }
  }

  // Queued input is delivered once, after the running turn.
  try {
    const at = chat.mark();
    chat.send({ type: 'acp_prompt', text: 'Reply with exactly the word: plum' });
    const queued = `acceptance-q-${Date.now()}`;
    chat.send({ type: 'queue_input', session_id: id, id: queued, text: 'Reply with exactly the word: cherry' });
    await chat.waitFor((f) => f.type === 'acp_turn_end', TURN_MS, at);
    await chat.waitFor((f, i) => f.type === 'acp_turn_end' && chat.frames.indexOf(f) > at && /cherry/i.test(chat.textSince(at)), TURN_MS, at);
    record('queue delivery', 'pass');
  } catch (e) { failed('queue delivery', ['Send a prompt', 'Queue a second prompt before it finishes'], e, chat); }

  // Reconnect: same chat, no duplicated input.
  try {
    chat.close();
    const again = new Chat(id);
    const ready = await again.connect();
    const r = await prompt(again, 'Reply with exactly the word: lemon');
    if (!/lemon/i.test(r.text)) throw new Error(`text ${sanitize(r.text)}`);
    const prompts = (ready.transcript_page?.items ?? []).filter(
      (i) => i?.message?.role === 'user' && i.message.content === 'Reply with exactly the word: pineapple',
    ).length;
    if (prompts !== 1) throw new Error(`the first prompt appears ${prompts} times in the replayed transcript`);
    record('reconnect', 'pass');
    chat.ws = again.ws; chat.frames = again.frames; chat.waiters = again.waiters;
  } catch (e) {
    failed('reconnect', ['Reload the window', 'Send "Reply with exactly the word: lemon"'], e, chat);
    if (chat.ws.readyState !== WebSocket.OPEN) await chat.connect();
  }

  // Native fork into a separate chat.
  const fork = chat.engine()?.capabilities?.fork ?? 'unavailable';
  if (fork === 'unavailable') record('fork', 'unavailable', { reason: 'the installed agent negotiated no fork' });
  else {
    try {
      const { id: forkId } = await http('POST', `/api/sessions/${encodeURIComponent(id)}/fork`, { text: '', occurrence: 0, whole: true });
      const forked = new Chat(forkId);
      await forked.connect();
      let r;
      try { r = await prompt(forked, 'Reply with exactly the word: guava'); } catch (e) { failed('fork', ['Agent menu → Fork chat', 'Send a prompt in the new chat'], e, forked); throw null; }
      if (!/guava/i.test(r.text)) throw new Error(`fork text ${sanitize(r.text)}`);
      const source = await prompt(chat, 'Reply with exactly the word: lime');
      if (!/lime/i.test(source.text)) throw new Error('the original chat stopped responding after the fork');
      forked.close();
      record('fork', 'pass', { scope: fork, fork_session: forkId });
    } catch (e) { if (e !== null) failed('fork', ['Agent menu → Fork chat', 'Send a prompt in each chat'], e, chat); }
  }

  for (const scenario of [
    'sign-in from settings', 'approvals', 'structured questions', 'images', 'early subagent cards',
    'scoped child streams', 'background activity after the parent result', 'terminal child states',
    'scroll position and bottom-follow', 'settings loading', 'sidebar elapsed time and spinner',
    'restart and ambiguous-dispatch recovery',
  ]) record(scenario, 'manual');
  chat.close();
  return { meta, results };
}

const engines = await http('GET', '/api/engines').then((e) => (Array.isArray(e) ? e : e.engines ?? e.instances ?? []));
const version = await http('GET', '/api/version').catch(() => null);
const report = { started_at: new Date().toISOString(), server: BASE, mira: version, agents: [] };
for (const instance of AGENTS) {
  console.log(`${instance}`);
  try {
    report.agents.push(await runAgent(instance, engines));
  } catch (e) {
    report.agents.push({ meta: { instance }, results: [{ scenario: 'harness', status: 'fail', reason: sanitize(e.message) }] });
    console.log(`  fail        harness — ${sanitize(e.message)}`);
  }
}
mkdirSync(OUT, { recursive: true });
writeFileSync(path.join(OUT, 'report.json'), JSON.stringify(report, null, 2));
const counts = report.agents.flatMap((a) => a.results).reduce((n, r) => ({ ...n, [r.status]: (n[r.status] ?? 0) + 1 }), {});
console.log(`\n${JSON.stringify(counts)} → ${path.join(OUT, 'report.json')}`);
process.exit(counts.fail ? 1 : 0);
