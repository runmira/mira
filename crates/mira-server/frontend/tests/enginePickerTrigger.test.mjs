import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, {act} from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const { EnginePicker, RuntimeProvider }=await bundle(`
export { EnginePicker } from './src/components/EnginePicker';
export { RuntimeProvider } from './src/state/RuntimeProvider';
`);
const window = new Window({url:'https://picker.mira.test'});
globalThis.window=window;globalThis.document=window.document;globalThis.IS_REACT_ACT_ENVIRONMENT=true;
test('selected model chip shows readable name beside both agent and provider names', async () => {
  for (const engine of [
    {kind:'agent',driver:'opencode',instance:'opencode',display_name:'OpenCode',model:'github-copilot/claude-sonnet-4.6',status:'ready'},
    {kind:'provider',instance:'openrouter',display_name:'OpenRouter',model:'openai/gpt-5.4',status:'ready'},
  ]) {
    const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
    await act(()=>root.render(React.createElement(RuntimeProvider,{scope:{backend:'fixture',account:engine.instance}},React.createElement(EnginePicker,{
      open:false,onOpenChange(){},engine,fallbackModel:'',engines:[],agents:[],onPickProvider(){},onPickAgent(){},onSetModelOption(){},
    }))));
    const visible=host.textContent;
    assert.equal(visible,engine.kind==='agent'?'OpenCodeClaude Sonnet 4.6':'OpenRouterGPT 5.4');
    assert.ok(!visible.includes('/'));
    await act(()=>root.unmount());host.remove();
  }
});
