import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { Composer } = await bundle(`export { Composer } from './src/components/Composer';`, {
  'src/components/composer/useComposerController': `export function useComposerController() { return {
    noticeItems: [], images: [], quotes: [], attachments: [], plans: [], questions: [], approvals: [],
    paletteGroups: [], paletteMatches: [], issuePicker: {open:false,onKeyDown:()=>false}, slash: {mode:'none'}, agentPicker: {modes:[]}, text: '', mentionRef: {current:null}, fileInputRef: {current:null},
  }; }`,
  'src/components/EnginePicker': `export function EnginePicker() { return null; }`,
});
function findPicker(element) {
  if (!element || typeof element !== 'object') return null;
  if (element.type?.name === 'EnginePicker') return element;
  for (const child of [element.props?.children].flat(Infinity)) {
    const found=findPicker(child); if (found) return found;
  }
  return null;
}
test('Composer preserves the chosen external instance through the actual picker adapter', () => {
  const picks=[];
  const tree=Composer({mode:'manual',cwd:'',model:'test',onPickAgent:(...args)=>picks.push(args),agentDriving:true,engine:{kind:'agent',driver:'codex',instance:'personal'},skills:[]});
  const picker=findPicker(tree);
  assert.ok(picker);
  picker.props.onPickAgent('codex','gpt-5','work');
  picker.props.onPickAgent('codex',null,'personal');
  assert.deepEqual(picks,[['codex','gpt-5','work'],['codex',null,'personal']]);
});
