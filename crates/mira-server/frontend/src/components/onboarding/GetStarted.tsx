/**
 * First run: nothing can answer a message yet. Instead of a warning strip,
 * offer the two ways Mira runs, side by side, with the one that needs no
 * setup first — a coding agent the user already has signed in.
 */
import { ArrowRight, KeyRound, Terminal } from 'lucide-react';
import type { AcpAgentStatus } from '../../types';
import { AgentIcon } from '../AgentIcon';
import { SubagentFace } from '../SubagentFace';

export function GetStarted({
  agents,
  onUseAgent,
  onAddProvider,
  onSetUpAgents,
}: {
  agents: AcpAgentStatus[];
  onUseAgent: (kind: string) => void;
  onAddProvider: () => void;
  onSetUpAgents: () => void;
}) {
  const ready = agents.filter((a) => a.state.state === 'ready' && a.cli_installed !== false);
  return (
    <div className="mx-auto flex min-h-full w-full max-w-2xl flex-col justify-center px-2 py-10">
      <div className="mb-3 flex items-center justify-center gap-1.5">
        <SubagentFace id="explore" face={{ color: '#2dd4bf', shape: 'round', eyes: 'wide', cheeks: true }} size={26} />
        <SubagentFace id="reviewer" face={{ color: '#f59e0b', shape: 'squircle', eyes: 'visor' }} size={26} />
        <SubagentFace id="coder" face={{ color: '#a78bfa', shape: 'squircle', eyes: 'happy' }} size={26} />
      </div>
      <h2 className="text-center text-[17px] font-medium tracking-tight text-foreground">
        How do you want to run Mira?
      </h2>
      <p className="mx-auto mt-1 max-w-[52ch] text-center text-[12.5px] leading-relaxed text-muted-foreground">
        Pick one to start — you can switch any time from the model picker, even mid-chat.
      </p>

      <div className="mt-5 grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col rounded-xl border border-border/70 bg-white/[0.025] p-4">
          <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
            <Terminal className="size-4 text-mira-blue" />
            An agent you already use
          </div>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">
            Claude Code or Codex drives the chat, on your own plan. Nothing to configure.
          </p>
          <div className="mt-3 flex flex-1 flex-col justify-end gap-1.5">
            {ready.length > 0 ? (
              ready.map((a) => (
                <button
                  key={a.kind}
                  type="button"
                  onClick={() => onUseAgent(a.kind)}
                  className="group flex items-center gap-2 rounded-lg border border-border/70 px-2.5 py-2 text-left text-[12.5px] transition-colors hover:border-mira-blue/50 hover:bg-mira-blue/[0.06]"
                >
                  <AgentIcon kind={a.kind} name={a.display_name} size="sm" tile={false} />
                  <span className="min-w-0 flex-1 truncate text-foreground/90">Start with {a.display_name}</span>
                  {a.auth && <span className="truncate text-[10.5px] text-muted-foreground/60">{a.auth}</span>}
                  <ArrowRight className="size-3.5 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
                </button>
              ))
            ) : (
              <>
                <code className="rounded-md bg-background/60 px-2 py-1.5 font-mono text-[11px] text-foreground/75">
                  npm i -g @anthropic-ai/claude-code
                </code>
                <button
                  type="button"
                  onClick={onSetUpAgents}
                  className="self-start text-[12px] text-muted-foreground underline underline-offset-2 hover:text-foreground"
                >
                  None found yet — see setup
                </button>
              </>
            )}
          </div>
        </div>

        <div className="flex flex-col rounded-xl border border-border/70 bg-white/[0.025] p-4">
          <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
            <KeyRound className="size-4 text-amber-400" />
            Your own API key
          </div>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">
            Mira&apos;s own harness with any model — Anthropic, OpenAI, Bedrock, OpenRouter, local — plus
            its subagents, goals and code review.
          </p>
          <div className="mt-3 flex flex-1 flex-col justify-end">
            <button
              type="button"
              onClick={onAddProvider}
              className="group flex items-center gap-2 rounded-lg border border-border/70 px-2.5 py-2 text-left text-[12.5px] transition-colors hover:border-amber-400/50 hover:bg-amber-400/[0.06]"
            >
              <span className="flex-1 text-foreground/90">Add a provider</span>
              <ArrowRight className="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
