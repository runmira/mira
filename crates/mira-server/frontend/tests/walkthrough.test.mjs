import assert from 'node:assert/strict';
import {test} from 'node:test';
import {bundle} from './helpers/bundle.mjs';
const {splitWalkthroughHunks,canRevertWalkthroughHunk,parseWalkthroughPr,restoreApprovals}=await bundle("export * from './src/lib/walkthrough';");
const patch='diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file\n@@ -20 +20 @@\n-before\n+after\n';
test('hunks retain file headers and no-newline markers without adjacent edits',()=>{
 const hunks=splitWalkthroughHunks(patch);assert.equal(hunks.length,2);
 assert.ok(hunks[0].includes('\\ No newline at end of file'));assert.ok(!hunks[0].includes('+after'));
 assert.ok(hunks[1].startsWith('diff --git a/a b/a\n--- a/a\n+++ b/a\n'));assert.ok(!hunks[1].includes('+new'));
 assert.equal(canRevertWalkthroughHunk(patch),true);
 for(const header of ['rename from a','new file mode 100644','deleted file mode 100644','old mode 100644','GIT binary patch']) assert.equal(canRevertWalkthroughHunk(header+'\n'+patch),false);
});
test('approvals survive reload only for the exact snapshot',()=>{
 const raw=JSON.stringify({files:[patch],approved:['0:0','0:1']});
 assert.deepEqual(restoreApprovals(raw,[patch]),['0:0','0:1']);
 assert.deepEqual(restoreApprovals(raw,[patch+'\n']),[]);
 assert.deepEqual(restoreApprovals('broken',[patch]),[]);
});
test('PR input accepts links and shorthand and rejects unrelated URLs',()=>{
 assert.deepEqual(parseWalkthroughPr('https://github.com/runmira/mira/pull/173'),{kind:'pr',owner:'runmira',repo:'mira',number:173});
 assert.deepEqual(parseWalkthroughPr('runmira/mira/173'),{kind:'pr',owner:'runmira',repo:'mira',number:173});
 assert.throws(()=>parseWalkthroughPr('https://example.com/runmira/mira/pull/173'));
 assert.throws(()=>parseWalkthroughPr('runmira/mira/0'));
});

import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
const browser=new Window({url:'https://mira.test'});
globalThis.window=browser;globalThis.document=browser.document;globalThis.localStorage=browser.localStorage;globalThis.IS_REACT_ACT_ENVIRONMENT=true;
let owner='session-a',calls=[],handler;
globalThis.__walkthroughOwner=()=>owner;
globalThis.__walkthroughCall=(...args)=>{calls.push(args);return handler(...args);};
const {GuidedWalkthrough}=await bundle("export {GuidedWalkthrough} from './src/components/GuidedWalkthrough';",{
 'src/api.ts':`export const walkthroughCall=(...args)=>globalThis.__walkthroughCall(...args);`,
 'src/state/RuntimeProvider.tsx':`export const useOptionalRuntime=()=>({sessionState:{get:()=>({active:globalThis.__walkthroughOwner()})}});`,
 'src/lib/diffPrefs.ts':`export const useDiffOptions=()=>({});`,
 'src/components/diffs/ReviewDiffView.tsx':`export const ReviewDiffView=({files})=><div data-testid="diff">{files.map(f=>f.filePath).join(',')}</div>;`,
});
function click(host,label){const b=[...host.querySelectorAll('button')].find(b=>b.textContent===label);assert.ok(b,`missing button ${label}`);return act(async()=>{b.click();await new Promise(resolve=>setImmediate(resolve));});}
async function flush(){await act(async()=>{await new Promise(resolve=>setImmediate(resolve));});}
const plan={files:[patch,patch.replaceAll('a/a','a/test').replaceAll('b/a','b/test')],stops:[{file_index:0,category:'core',summary:'Update the core behavior.',risks:['Deleted validation at line 1']},{file_index:1,category:'tests',summary:'Verify the behavior.',risks:[]}]};
test('walks stops and hunks, saves exact approvals, asks in place, and pins revert to owner',async()=>{
 calls=[];owner='session-a';localStorage.clear();
 handler=async(action)=>action===''?plan:action==='/question'?{answer:'The old check was removed.'}:{ok:true};
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);let changed=0;
 await act(()=>root.render(React.createElement(GuidedWalkthrough,{onChanged(){changed++;},onClose(){}})));
 await click(host,'Start walkthrough');await flush();
 assert.ok(host.textContent.includes('Stop 1 of 2'));assert.ok(host.textContent.includes('Deleted validation'));
 await click(host,'Approve hunk');assert.ok(host.textContent.includes('1/4 reviewed'));
 await click(host,'Next hunk');await click(host,'Approve hunk');assert.ok(host.textContent.includes('2/4 reviewed'));
 await click(host,'Next stop');assert.ok(host.textContent.includes('Stop 2 of 2'));
 const question=host.querySelector('[aria-label="Question about this stop"]');
 await act(()=>{const setter=Object.getOwnPropertyDescriptor(browser.HTMLInputElement.prototype,'value').set;setter.call(question,'Why remove it?');question.dispatchEvent(new browser.Event('input',{bubbles:true}));});
 // Use the form submission so this also exercises keyboard Enter submission.
 await act(async()=>{question.closest('form').dispatchEvent(new browser.Event('submit',{bubbles:true,cancelable:true}));await new Promise(resolve=>setImmediate(resolve));});await flush();
 assert.ok(host.textContent.includes('The old check was removed.'));
 await click(host,'Revert hunk');await click(host,'Confirm revert');await flush();
 assert.equal(changed,1);const revert=calls.find(c=>c[0]==='/revert');assert.equal(revert[2],'session-a');assert.equal(revert[1].file_index,1);assert.equal(revert[1].expected_patch,plan.files[1]);
 assert.ok(host.textContent.includes('Hunk reverted'));assert.ok(host.querySelector('[data-testid="diff"]'));
 await act(()=>root.unmount());host.remove();
});
test('late plans cannot cross chat ownership',async()=>{
 calls=[];owner='session-a';let resolvePlan;handler=()=>new Promise(resolve=>resolvePlan=resolve);
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const props={onChanged(){},onClose(){}};
 await act(()=>root.render(React.createElement(GuidedWalkthrough,props)));
 await click(host,'Start walkthrough');owner='session-b';await act(()=>root.render(React.createElement(GuidedWalkthrough,props)));
 resolvePlan(plan);await flush();assert.ok(!host.textContent.includes('Update the core behavior.'));assert.ok(host.textContent.includes('Start walkthrough'));
 await act(()=>root.unmount());host.remove();await browser.happyDOM.abort();
});
