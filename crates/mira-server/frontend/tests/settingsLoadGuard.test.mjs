import assert from 'node:assert/strict';
import { test } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const window=new Window({url:'https://mira.test'});
globalThis.window=window;globalThis.document=window.document;
const { GeneralSection, EMPTY_DRAFT }=await bundle(`
export { GeneralSection } from './src/components/settings/GeneralSection';
export { EMPTY_DRAFT } from './src/components/settings/types';
`,{
 'src/state/RuntimeProvider':`export const useOptionalRuntime=()=>null; export const useAppRuntime=()=>({resources:{saveSettings:async()=>{}}});`,
 'src/state/resources/hooks':`export const useResource=()=>globalThis.settingsResource;`,
});
function switches(resource){
 globalThis.settingsResource=resource;
 const dom=document.createElement('div');dom.innerHTML=renderToStaticMarkup(React.createElement(GeneralSection,{draft:EMPTY_DRAFT,setDraft(){}}));
 const labels=['Auto resume after a limit resets','Keep the screen awake while a session runs'];
 return {dom,controls:labels.map(label=>dom.querySelector(`[aria-label="${label}"]`))};
}
test('server session switches stay disabled until settings are known, including failed loading',()=>{
 for(const resource of [{data:undefined,error:null},{data:undefined,error:new Error('unavailable')}]){
  const {controls}=switches(resource);assert.ok(controls.every(control=>control?.hasAttribute('disabled')));
 }
 const {dom,controls}=switches({data:{sessions:{auto_resume_after_limit:true,keep_awake_while_running:true}},error:null});
 assert.ok(controls.every(control=>!control.hasAttribute('disabled')&&control.getAttribute('aria-checked')==='true'));
 assert.ok(!dom.textContent.includes("Couldn't load these settings"));
 assert.ok(switches({data:undefined,error:new Error('unavailable')}).dom.textContent.includes("Couldn't load these settings"));
});
