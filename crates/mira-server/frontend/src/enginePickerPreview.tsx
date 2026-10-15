/**
 * Dev-only preview of the composer's model picker with fixture engines that
 * mirror a real machine: four providers, three installed agents (Codex is
 * this chat's), two that are not installed and one switched off. Nothing
 * reaches a server. Open /engine-picker-preview.html on the Vite dev server.
 */
import { LazyMotion, domAnimation } from 'framer-motion';
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import type { EngineSnapshot } from './api';
import { EnginePicker } from './components/EnginePicker';
import { acpOptionsToDescriptors } from './lib/acpOptions';
import { RuntimeProvider } from './state/RuntimeProvider';
import type { AcpAgentStatus, AcpConfigOption, SessionEngine } from './types';
import './styles.css';

const provider = (instance: string, display_name: string, models: string[]): EngineSnapshot =>
  ({ instance, driver: 'native', flavor: 'native', enabled: true, display_name, state: { state: 'ready' }, models: models.map((id) => ({ id })) }) as EngineSnapshot;
const agent = (instance: string, display_name: string, status: Partial<AcpAgentStatus>, enabled = true): EngineSnapshot => {
  const state = status.state ?? { state: 'ready' };
  return {
    instance, driver: instance, flavor: 'external', enabled, display_name, state, models: [],
    agent: { kind: instance, display_name, state, auth_method_ids: [], launch: instance, ...status } as AcpAgentStatus,
  } as EngineSnapshot;
};
const engines: EngineSnapshot[] = [
  provider('anthropic', 'Anthropic', ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5']),
  provider('groq', 'Groq', ['llama-4-maverick']),
  provider('ollama', 'Ollama', ['qwen3-coder:30b']),
  provider('openrouter', 'OpenRouter', [
    'openai/gpt-6.1-sol', 'openai/gpt-6-luna', 'anthropic/claude-opus-5-5', 'anthropic/claude-sonnet-5-5',
    'google/gemini-4-argon', 'deepseek/deepseek-v4-flash', 'qwen/qwen3-coder', 'moonshotai/kimi-k3',
    'x-ai/grok-4.7', 'mistralai/devstral-2', 'meta-llama/llama-4-maverick', 'z-ai/glm-5', 'minimax/minimax-m3',
    'nvidia/nemotron-3-ultra', 'openai/gpt-oss-240b']),
  agent('claude-code', 'Claude Code', { auth: 'Claude Max · you@example.com', version: '2.1.283', transport: 'native' }),
  // Codex reports its app-server user agent where other agents report a version.
  agent('codex', 'Codex', { models: [
    { value: 'gpt-6.1-sol', label: 'GPT-6.1-Sol', is_default: true }, { value: 'gpt-6-astra', label: 'GPT-6-Astra' }],
    auth: 'you@example.com · plus', version: 'mira-probe/0.162.0-alpha.17.2 (Mac OS 26.3.0; arm64) Apple_Terminal/455.1 (mira-probe; 0.6.2)', transport: 'native' }),
  agent('opencode', 'OpenCode', { version: '2.0.22' }),
  agent('antigravity', 'Antigravity', { state: { state: 'not_found', looked_for: 'antigravity' } }),
  agent('grok', 'Grok Build', { state: { state: 'not_found', looked_for: 'grok' } }),
  agent('cursor', 'Cursor', { state: { state: 'not_found', looked_for: 'cursor-agent' } }, false),
];

// Seed a couple of recent OpenRouter picks so the Recent group shows.
try {
  localStorage.setItem('mira.picker.recent.v1', JSON.stringify({ 'provider:openrouter': ['moonshotai/kimi-k3', 'anthropic/claude-sonnet-5-5'] }));
} catch { /* storage unavailable */ }

function Preview() {
  const [open, setOpen] = useState(true);
  const [model, setModel] = useState('gpt-6-astra');
  const [opts, setOpts] = useState<Record<string, string>>({ reasoning_effort: 'low', service_tier: '' });
  const config: AcpConfigOption[] = [
    { id: 'model', name: 'Model', category: 'model', current: model, values: [
      { value: 'gpt-6.1-sol', name: 'GPT-6.1-Sol' }, { value: 'gpt-6-astra', name: 'GPT-6-Astra' },
      { value: 'gpt-6-sol', name: 'GPT-6-Sol' }, { value: 'gpt-6-luna', name: 'GPT-6-Luna' }] },
    { id: 'reasoning_effort', name: 'Reasoning', category: 'reasoning_effort', current: opts.reasoning_effort,
      values: ['low', 'medium', 'high', 'xhigh'].map((value) => ({ value, name: value })) },
    { id: 'service_tier', name: 'Speed', category: 'service_tier', current: opts.service_tier,
      values: [{ value: '', name: 'Default' }, { value: 'priority', name: 'Fast' }] },
  ];
  const engine: SessionEngine = {
    kind: 'agent', driver: 'codex', instance: 'codex', display_name: 'Codex', model, status: 'ready',
    capability_source: 'live', agent_version: 'mira/0.162.0-alpha.17.2 (Mac OS 26.3.0; arm64) Apple_Terminal/455.1 (mira; 0.6.2)',
    capabilities: { asynchronous_questions: true, background_work: true, native_goals: false, steering: 'native', image_input: true,
      live_model_switch: true, live_mode_switch: true, fork: 'turn', native_rollback: true, native_snapshot: false, cancellation: true, stop_behavior: 'current_turn' },
  };
  return (
    <LazyMotion features={domAnimation}>
      <main className="min-h-screen bg-background p-6 text-foreground">
        <button className="rounded-lg border border-border px-3 py-1.5 text-sm" onClick={() => setOpen(true)}>Open model picker</button>
        <EnginePicker
          open={open} onOpenChange={setOpen} engine={engine} fallbackModel="" engines={engines} agents={[]}
          agentConfig={config} agentDescriptors={acpOptionsToDescriptors(config)?.filter((o) => o.id !== 'model') ?? []}
          onPickProvider={() => {}} onPickAgent={(_d, m) => m && setModel(m)} onSetModelOption={(id, v) => setOpts((p) => ({ ...p, [id]: v }))}
          onConfigureAgents={() => {}}
        />
      </main>
    </LazyMotion>
  );
}
createRoot(document.getElementById('root')!).render(<RuntimeProvider scope={{ backend: 'fixture', account: 'fixture' }}><Preview /></RuntimeProvider>);
