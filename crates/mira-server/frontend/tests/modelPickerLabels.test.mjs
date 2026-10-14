import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { modelPickerLabel }=await bundle(`export { modelPickerLabel } from './src/lib/models';`);
test('model labels drop routing prefixes and preserve branding and versions', () => {
  assert.equal(modelPickerLabel('github-copilot/claude-sonnet-4.6','github-copilot/claude-sonnet-4.6'),'Claude Sonnet 4.6');
  assert.equal(modelPickerLabel('openrouter/~openai/gpt-5.4'),'GPT 5.4');
  assert.equal(modelPickerLabel('deepseek/deepseek-v4-flash'),'DeepSeek V4 Flash');
  assert.equal(modelPickerLabel('openai/gpt-5.4','My custom model'),'My Custom Model');
  const id='github-copilot/gpt-5.4';modelPickerLabel(id);assert.equal(id,'github-copilot/gpt-5.4');
});
