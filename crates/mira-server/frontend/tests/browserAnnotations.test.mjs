import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { notesMarkdown, describeAt, createDraftService } = await bundle(`
export { notesMarkdown } from './src/lib/browserNotes';
export { describeAt } from './src/lib/browserAnnotation';
export { createDraftService } from './src/state/draftService';
`, { 'src/lib/richComposerDrafts': `
const drafts = new Map();
export const cachedDraft = key => drafts.get(key) ?? {text:'',attachments:[],images:[]};
export const changeRichDraft = (key,draft) => drafts.set(key,draft);
export const clearRichDraft = key => drafts.delete(key);
export const restoreRichDraft = async key => cachedDraft(key);
export const flushRichDrafts = async () => {};
` });
test('notes describe multiple pages and preserve screenshot order', () => {
  assert.equal(notesMarkdown([]), '');
  const body = notesMarkdown([{url:'https://one.test',target:'the "Save" button',comment:'Move this'}, {url:'https://two.test',target:null,comment:'Explain this'}]);
  assert.match(body, /1\. At the "Save" button \(https:\/\/one.test\): Move this/);
  assert.match(body, /2\. At the marked spot \(https:\/\/two.test\): Explain this/);
  assert.match(body, /screenshots mark each one, in order/);
});
test('area descriptions inspect the centre and normalize page whitespace', async () => {
  const original = globalThis.fetch;
  globalThis.fetch = async (_url, options) => {
    const {expression} = JSON.parse(options.body).action;
    const document = {elementFromPoint(x,y) {
      assert.equal(x,40); assert.equal(y,35);
      return {tagName:'BUTTON',getAttribute:()=>null,innerText:'Save\n  changes'};
    }};
    return {ok:true,json:async()=>({text:Function('document',`return ${expression}`)(document)})};
  };
  try { assert.equal(await describeAt({x:10,y:20,w:60,h:30}), 'the "Save changes" button'); }
  finally { globalThis.fetch = original; }
});
test('failed inspection leaves notes usable without a target', async () => {
  const original=globalThis.fetch; globalThis.fetch=async()=>{throw Error('offline');};
  try { assert.equal(await describeAt({x:10,y:20}),null); } finally {globalThis.fetch=original;}
});
test('notes stay with their draft and survive late delivery acknowledgments', async () => {
  const drafts=createDraftService({backend:'browser-notes-test',account:'a'});
  const a=drafts.key('a'),b=drafts.key('b');
  const note={id:'n1',url:'https://one.test',target:null,comment:'Fix it',thumb:'crop',data:{media_type:'image/png',data:'marked'}};
  drafts.update(a,{...drafts.get(a),notes:[note]},true); drafts.markDelivery(a,'input-1');
  drafts.update(a,{...drafts.get(a),notes:[note,{...note,id:'n2'}]},true); drafts.confirmDelivery('input-1');
  assert.equal(drafts.get(b).notes,undefined);
  assert.deepEqual(drafts.get(a).notes.map(n=>n.id),['n1','n2']);
  assert.equal((await drafts.restore(a)).notes.length,2);
  drafts.markDelivery(a,'input-2');drafts.confirmDelivery('input-2');assert.equal(drafts.get(a).notes,undefined);
  drafts.dispose();
});
