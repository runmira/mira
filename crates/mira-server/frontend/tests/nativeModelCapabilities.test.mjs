import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { agentAcceptsImages, acpOptionsToDescriptors } = await bundle("export { agentAcceptsImages, acpOptionsToDescriptors } from './src/lib/acpOptions';");
const catalog = current => [{id:'model',category:'model',current,values:[
  {value:'text',name:'Text model',capabilities:{inputModalities:['text']}},
  {value:'vision',name:'Vision model',capabilities:{inputModalities:['text','image']}},
  {value:'legacy',name:'Legacy model'},
]}];
test('image availability follows the selected model and tolerates older catalogs', () => {
  assert.equal(agentAcceptsImages(catalog('text')),false);
  assert.equal(agentAcceptsImages(catalog('vision')),true);
  assert.equal(agentAcceptsImages(catalog('legacy')),true);
  assert.equal(agentAcceptsImages([]),true);
});
test('reasoning and speed controls contain only advertised choices including the null tier default', () => {
  const descriptors=acpOptionsToDescriptors([
    {id:'reasoning_effort',name:'Reasoning',category:'reasoning_effort',current:'low',values:[{value:'low',name:'Low'},{value:'high',name:'High'}]},
    {id:'service_tier',name:'Speed',category:'service_tier',current:null,values:[{value:'',name:'Default'},{value:'priority',name:'Fast'}]},
  ]);
  assert.deepEqual(descriptors.map(d=>d.options.map(v=>v.value)),[['low','high'],['','priority']]);
  assert.deepEqual(descriptors.map(d=>d.label),['Reasoning','Speed']);
});
