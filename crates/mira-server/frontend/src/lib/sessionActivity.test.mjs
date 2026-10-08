import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from 'typescript';
const output=ts.transpileModule(await readFile(new URL('./sessionActivity.ts',import.meta.url),'utf8'),{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText;
const {applySessionActivity:apply}=await import(`data:text/javascript;base64,${Buffer.from(output).toString('base64')}`);
const snapshot=(epoch,revision,running)=>({snapshot:{epoch,revision,running}});
test('completion cannot be overwritten by stale running or stale snapshots',()=>{
 let state=apply(null,snapshot('boot',1,['a','b']));
 state=apply(state,{epoch:'boot',revision:2,session_id:'a',running:false});
 assert.deepEqual([...state.running],['b']);
 assert.equal(apply(state,{epoch:'boot',revision:1,session_id:'a',running:true}),state);
 assert.equal(apply(state,snapshot('boot',1,['a','b'])),state);
});
test('reconnect and server restart snapshots clear ghosts across every chat',()=>{
 let state=apply(null,snapshot('old',900,['a','b']));
 state=apply(state,snapshot('new',1,[]));assert.equal(state.running.size,0);
 assert.equal(apply(state,{epoch:'old',revision:999,session_id:'a',running:true}),state);
 state=apply(state,{epoch:'new',revision:2,session_id:'c',running:true});
 state=apply(state,snapshot('new',5,[]));assert.equal(state.running.size,0);
});
