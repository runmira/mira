import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const storage = new Map();
globalThis.localStorage = {getItem:key=>storage.get(key)??null, setItem:(key,value)=>storage.set(key,value)};
const url = source => `data:text/javascript;base64,${Buffer.from(ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText).toString('base64')}`;
const legacyURL = url(readFileSync(new URL('./composerDrafts.ts',import.meta.url),'utf8'));
const legacy = await import(legacyURL);
const disk = new Map(); const reads = new Map();
const transaction = (name,mode) => {
  const tx = {objectStore:() => ({ get:key=>{ const req={}; reads.set(key,()=>{req.result=disk.get(key);req.onsuccess();}); return req; } })};
  if(mode==='readwrite') queueMicrotask(()=>tx.onabort?.()); // simulate quota failure; no fake successful persistence
  return tx;
};
globalThis.indexedDB = {open:()=>{const request={};queueMicrotask(()=>{request.result={transaction}; request.onsuccess();});return request;}};
const source = readFileSync(new URL('./richComposerDrafts.ts',import.meta.url),'utf8').replace("'./composerDrafts'",JSON.stringify(legacyURL));
const drafts = await import(url(source));
const file = {path:'notes.txt',content:'hello',bytes:5};
const image = {media_type:'image/png',data:'test-image'};
const empty = {text:'',attachments:[],images:[]};
async function startRead(key,stored) { disk.set(key,{...stored,key,at:Date.now()});const promise=drafts.restoreRichDraft(key);for(let i=0;i<10&&!reads.has(key);i++)await Promise.resolve();return {promise,release:()=>reads.get(key)()}; }
test('legacy clear cancels pending keystrokes instead of resurrecting sent text',()=>{legacy.queueDraft('sent','old text');legacy.clearDraft('sent');legacy.flushDrafts();assert.equal(legacy.readDraft('sent'),'');});
test('late disk hydration preserves new text and restores untouched files/images',async()=>{
 const read=await startRead('late',{text:'old',attachments:[file],images:[image]});
 drafts.changeRichDraft('late',{...empty,text:'typing now'}); read.release();
 assert.deepEqual(await read.promise,{text:'typing now',attachments:[file],images:[image]});
 drafts.clearRichDraft('late'); await drafts.flushRichDrafts();
});
test('send while restore is pending fences old text and attachments',async()=>{
 const read=await startRead('cleared',{text:'old',attachments:[file],images:[image]});
 drafts.clearRichDraft('cleared'); read.release(); assert.deepEqual(await read.promise,empty);await drafts.flushRichDrafts();
});
test('attachment-only drafts remain isolated and survive storage failure in memory',async()=>{
 drafts.changeRichDraft('chat-a',{...empty,attachments:[file],images:[image]});
 drafts.changeRichDraft('chat-b',{...empty,text:'other chat'});
 assert.equal(drafts.cachedDraft('chat-b').images.length,0);
 assert.equal(drafts.cachedDraft('chat-a').text,'');
 const a=await startRead('chat-a',empty); a.release(); const b=await startRead('chat-b',empty); b.release();
 await Promise.all([a.promise,b.promise]); await drafts.flushRichDrafts();
 assert.deepEqual(drafts.cachedDraft('chat-a').images,[image]);
 drafts.clearRichDraft('chat-a');drafts.clearRichDraft('chat-b');await drafts.flushRichDrafts();
});
