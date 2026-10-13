import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { sessionFetch } = await bundle(`export { sessionFetch } from './src/api/sessionScope';`);
test('concurrent repository requests retain their captured owner and existing headers', async () => {
  const original=globalThis.fetch;const requests=[];
  globalThis.fetch=(url,options)=>new Promise(resolve=>requests.push({url,options,resolve}));
  try {
    const a=sessionFetch('/api/git/commit',{method:'POST',headers:{'content-type':'application/json'},body:'{}'},'a');
    const b=sessionFetch('/api/context',undefined,'b');
    assert.equal(requests[0].options.headers.get('x-mira-session'),'a');
    assert.equal(requests[1].options.headers.get('x-mira-session'),'b');
    assert.equal(requests[0].options.headers.get('content-type'),'application/json');
    requests[1].resolve(new Response('b'));requests[0].resolve(new Response('a'));
    assert.equal(await (await a).text(),'a');assert.equal(await (await b).text(),'b');
  } finally {globalThis.fetch=original;}
});
