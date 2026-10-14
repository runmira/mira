// OpenCode v2 promise-plugin API. Runs inside the ACP-owned server, so prompt
// admission joins that exact conversation without a second ACP turn or cancel.
import { timingSafeEqual } from 'node:crypto';
const bridges = new Map();
export default {
  id: 'mira.steering',
  async setup(context) {
    const { port, authFile } = context.options;
    if (!Number.isInteger(port) || port < 1 || port > 65535 || typeof authFile !== 'string' || typeof context.session?.prompt !== 'function') throw new Error('Invalid Mira steering bridge configuration');
    const token = (await Bun.file(authFile).text()).trim();
    if (token.length < 32) throw new Error('Invalid Mira steering bridge credential');
    const expected = Buffer.from(`Basic ${Buffer.from(`opencode:${token}`).toString('base64')}`);
    let bridge = bridges.get(port);
    if (!bridge) {
      const contexts = new Set();
      const receipts = new Map();
      const server = Bun.serve({
        hostname: '127.0.0.1', port, maxRequestBodySize: 16 * 1024 * 1024,
        async fetch(request) {
          const supplied = Buffer.from(request.headers.get('authorization') ?? '');
          if (supplied.length !== expected.length || !timingSafeEqual(supplied, expected)) return new Response('Unauthorized', {status:401});
          if (request.headers.has('origin')) return new Response('Forbidden', {status:403});
          const path = new URL(request.url).pathname;
          if (request.method === 'GET' && path === '/health') return Response.json({bridge:'mira.steering.v2'});
          if (request.method !== 'POST' || path !== '/mira/steer') return new Response('Not found', {status:404});
          let input;
          try { input = await request.json(); } catch { return new Response('Invalid input', {status:400}); }
          if (!input || typeof input !== 'object' || typeof input.sessionID !== 'string' || !input.sessionID || typeof input.id !== 'string' || !input.id.startsWith('msg_mira_steer_') || typeof input.prompt?.text !== 'string' || !Array.isArray(input.prompt?.files) || input.prompt.files.some(file => typeof file?.uri !== 'string' || !/^data:image\/[a-zA-Z0-9.+-]+;base64,[a-zA-Z0-9+/=]*$/.test(file.uri))) return new Response('Invalid input', {status:400});
          const key = `${input.sessionID}:${input.id}`;
          let receipt = receipts.get(key);
          if (!receipt) {
            const owner = contexts.values().next().value;
            if (!owner) return new Response('Bridge stopped', {status:503});
            receipt = Promise.resolve().then(() => owner.session.prompt({sessionID:input.sessionID,id:input.id,text:input.prompt.text,files:input.prompt.files,delivery:'steer'})).then(() => true, () => false);
            receipts.set(key, receipt);
            // Completed entries are expendable; in-flight admissions stay deduplicated.
            void receipt.then(() => { if (receipts.size > 1000) receipts.delete(key); });
          }
          return await receipt ? Response.json({accepted:true,id:input.id}) : new Response('Steering admission failed', {status:502});
        },
      });
      bridge = {server, contexts};
      bridges.set(port, bridge);
    }
    bridge.contexts.add(context);
    return () => {
      bridge.contexts.delete(context);
      if (!bridge.contexts.size) { bridge.server.stop(true); bridges.delete(port); }
    };
  },
};
