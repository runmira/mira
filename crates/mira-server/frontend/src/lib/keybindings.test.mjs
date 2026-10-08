import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from 'typescript';
let source=await readFile(new URL('./keybindings.ts',import.meta.url),'utf8');
source=source.replace("import { useSyncExternalStore } from 'react';",'const useSyncExternalStore=(_subscribe,snapshot)=>snapshot();');
const output=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText;
const saved=new Map();globalThis.localStorage={getItem:key=>saved.get(key)??null,setItem:(key,value)=>saved.set(key,value)};
const engine=await import(`data:text/javascript;base64,${Buffer.from(output).toString('base64')}`);
const event=(key,metaKey=true,altKey=false)=>({key,metaKey,ctrlKey:false,shiftKey:false,altKey});
const match=(e,context={})=>engine.resolveShortcutCommand(e,engine.useKeybindings(),{platform:'MacIntel',context});
test('removed default stays unassigned and reset restores it',()=>{
 engine.setCommandKeybindings('sidebar.toggle',[]);assert.equal(match(event('b')),null);
 assert.equal(JSON.parse(saved.get('mira.keybindings.v1')).find(rule=>rule.command==='sidebar.toggle').when,'false');
 engine.resetKeybindingCommand('sidebar.toggle');assert.equal(match(event('b')),'sidebar.toggle');
});
test('rebinding overrides defaults and survives export and import',()=>{
 engine.setCommandKeybindings('sidebar.toggle',[{command:'sidebar.toggle',key:'mod+alt+s',when:'!terminalFocus'}]);
 assert.equal(match(event('b')),null);assert.equal(match(event('s',true,true)),'sidebar.toggle');
 const exportJson=JSON.stringify(engine.getCustomKeybindingRules());engine.resetKeybindingCommand('sidebar.toggle');
 assert.equal(engine.importCustomKeybindingRules(exportJson),null);assert.equal(match(event('s',true,true)),'sidebar.toggle');
 assert.equal(match(event('s',true,true),{terminalFocus:true}),null);engine.resetKeybindingCommand('sidebar.toggle');
});
test('number shortcuts distinguish chats from panel tabs and approval letters yield to typing',()=>{
 assert.equal(match(event('1',false,true)),'chat.slot1');assert.equal(match(event('1',true,true)),'panel.slot1');
 assert.equal(match(event('y',false),{approvalOpen:true,editableFocus:true}),null);
 assert.equal(match(event('y',false),{approvalOpen:true}),'approval.accept');
});
