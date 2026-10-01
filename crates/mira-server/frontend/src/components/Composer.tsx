import { useEffect, useMemo, useRef, useState } from 'react';
import { ImageLightbox } from './ImageLightbox';
import { AnimatePresence, motion } from 'framer-motion';
import {
  ArrowUp,
  Camera,
  Circle,
  Cloud,
  Copy,
  File as FileIcon,
  Folder,
  GitBranch,
  Lightbulb,
  Link,
  Loader,
  Monitor,
  Paperclip,
  Plus,
  Square,
  Target,
  X,
} from 'lucide-react';
import {
  ArrowDown as PhArrowDown,
  ArrowLeft as PhArrowLeft,
  ArrowRight as PhArrowRight,
  ArrowUp as PhArrowUp,
  Check as PhCheck,
  Lightbulb as PhLightbulb,
  PenLine,
  Sparkle,
  Trash,
} from 'lucide-react';
import { createWorktree, getGitStatus, putCwd, readFile, type EngineSnapshot, type GitStatusView, type OptionDescriptor } from '../api';
import { PREF_KEYS, useBoolPref } from '../lib/prefs';
import type { ApprovalScope, AskUserAnswer, AskUserProposal, DiffLine, DiffPreview, EnvironmentInfo, EnvironmentStatus, Goal, Mode, PlanProposal, PlanStep, RateLimitReading, ToolCall, UsageTotals } from '../types';
import type { AcpConfigOption, AcpSessionMode, SessionEngine } from '../types';
import type { AskUserDecision } from './AskUserCard';
import { ApprovalChoices } from './ApprovalDialog';
import type { AcpAgentStatus } from '../types';
import { EnginePicker } from './EnginePicker';
import { UsageRing, type UsageRingData } from './UsageRing';
import { agentRequestHeadline, agentRequestOf } from '../lib/agentRequest';
import { partsNeedingApproval, splitShellCommand } from '../lib/shellParts';
import { mapPosturesToModes, MIRA_MODE_TO_POSTURE } from '../lib/agentPostures';
import { infoFor } from './ToolGroup';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { FilePicker } from './FilePicker';
import { customToCommand, filterCommands, groupCommands, originIconSrc, slashState, type PaletteGroup, type PaletteSkill, type SlashCommand } from './commands';
import type { CommandInfo, Origin } from '../api';
import { MentionInput, type MentionInputHandle } from './MentionInput';
import { cn } from '@/lib/utils';
import { ATTACH_FILE_EVENT } from '@/lib/attachBridge';

const MODES: { value: Mode; label: string; desc: string }[] = [
  { value: 'plan',   label: 'Plan only',       desc: 'Reads and searches. No edits, no commands.' },
  { value: 'manual', label: 'Ask each time',   desc: 'Prompt before every edit or command.' },
  { value: 'auto',   label: 'Auto edits',      desc: 'Auto-approve edits. Prompt on commands.' },
  { value: 'edit',   label: 'Auto everything', desc: 'Edits + commands run unless a rule blocks.' },
  { value: 'yolo',   label: 'Yolo',            desc: 'No gating at all.' },
];

type Props = {
  disabled: boolean;
  busy: boolean;
  mode: Mode;
  model: string;
  /** Configured routing provider (openrouter, openai, groq, …). Shown as
   *  the "Provider" row in the model picker so users see who's actually
   *  serving the request, not the model family extracted from the id. */
  providerName?: string | null;
  cwd: string;
  /** Running session totals. Rendered as the composer's footer readout;
   *  cost is priced against the currently-selected model. Null (or all-zero)
   *  hides the readout entirely — no "$0.00" for a fresh session. */
  usage: UsageTotals | null;
  /** The provider's latest rate-limit reading, when it reports one. */
  rateLimit?: RateLimitReading | null;
  onSend: (text: string, images?: ImageData[]) => void;
  onSetMode: (m: Mode) => void;
  onSetModel: (m: string, instance?: string | null) => void;
  /** What drives this session, from the server. The picker and the mode
   *  control render from this alone. */
  engine?: SessionEngine | null;
  /** Backend list from `GET /api/engines` — native providers + catalogs. */
  engines?: EngineSnapshot[] | null;
  /** Health of every agent, for the picker's Agents rail. */
  agents?: AcpAgentStatus[] | null;
  agentsChecking?: boolean;
  onCheckAgents?: () => void;
  /** The running agent's config options (its model list included). */
  agentConfig?: AcpConfigOption[] | null;
  /** The running agent's other options, projected for rendering. */
  agentDescriptors?: OptionDescriptor[] | null;
  /** Hand the chat to a provider (instance may be null for the default). */
  onPickProvider?: (instance: string | null, model: string) => void;
  /** Hand the chat to an agent, optionally with a model. */
  onPickAgent?: (driver: string, model: string | null) => void;
  /** Open Settings → Agents. */
  onConfigureAgents?: () => void;
  sessionId?: string | null;
  onAgentCompact?: () => void;
  onAgentFork?: () => void;
  onAgentReverted?: () => void;
  /** Pick the agent's mode through Mira's own picker. The pick is confirmed
   *  in the universal approval dialog (a mode is a grant of standing
   *  authority), then applied to the agent. */
  onPickAgentMode?: (m: Mode) => void;
  /** An external agent drives this session. Mira's own mode picker is then
   *  dead UI — turns go to the agent, not the harness — so the bar shows the
   *  agent's posture instead. One mode control, never two side by side. */
  agentDriving?: boolean;
  /** Session modes an external agent offers, or null. */
  onAcpModes?: AcpSessionMode[] | null;
  /** The agent's active mode id. */
  onAcpCurrentMode?: string | null;
  /** Apply one advertised model option (e.g. `reasoning_effort`,
   *  `service_tier`). The server validates the id. */
  onSetModelOption: (id: string, value: string) => void;
  onOpenPicker: () => void;
  /** Called after a successful in-composer cwd switch (Composer's own
   *  quick-switch dropdown, not the FolderPicker dialog). `sessionId` is
   *  the fresh slot the server built for the new folder — App attaches
   *  its WS to it so the new Ready lands in the transcript. */
  onCwdSwitched?: (path: string, sessionId?: string) => void;
  /** Remote environments for this session (null until the server replies). */
  environment?: EnvironmentStatus | null;
  environments?: EnvironmentInfo[];
  /** Latest progress line while a switch runs; null when idle. */
  envSwitching?: string | null;
  onSwitchEnvironment?: (target: string) => void;
  onInterrupt: () => void;
  onNewChat: () => void;
  onOpenSettings: () => void;
  onRunReview: (args: string) => void;
  /** Kick off an autonomous run. `maxIterations` is optional; server
   *  defaults to `mira_harness::DEFAULT_MAX_ITERATIONS` when omitted. */
  onSetGoal: (condition: string, maxIterations?: number) => void;
  /** Drop the standing goal (if any). */
  onClearGoal: () => void;
  /** `/compact [focus]`: summarize the conversation now. */
  onCompact: (focus: string) => void;
  /** Session's standing `/goal`, if any. Renders a purple chip at the
   *  top of the composer while active — mirrors the Plan chip pattern
   *  so users know autonomy is on. */
  goal: Goal | null;
  onRemember: (scope: 'user' | 'project', text: string) => Promise<string>;
  onUndo: (count: number) => Promise<string>;
  /** Loaded skill roster from `/api/skills`. Rendered inline in the
   *  slash palette after the built-in commands; picking `/<name>`
   *  fires a canned "use the `<name>` skill." user message which the
   *  model turns into a `Skill` tool call. Empty array = no skills or
   *  the roster hasn't loaded yet — palette still works. */
  skills: PaletteSkill[];
  /** Active approval waiting for the user's Y/N decision. When set,
   *  the Composer grows upward to show the approval UI instead of the
   *  text input. */
  /** Context, limits and spend for the usage ring. */
  usageRing?: UsageRingData | null;
  pendingApproval?: PendingApproval | null;
  /** How many approvals are waiting, including the one shown. */
  pendingApprovalCount?: number;
  /** Allow every waiting request once. */
  onAllowAllPending?: () => void;
  /** Active plan proposal waiting for the user to approve/cancel. */
  pendingPlan?: { callId: string; proposal: PlanProposal } | null;
  /** Active ask_user proposal waiting for answers. */
  pendingAskUser?: { callId: string; proposal: AskUserProposal } | null;
  /** Approve/deny a pending tool call. */
  onDecide?: (callId: string, allow: boolean, scope?: ApprovalScope) => void;
  /** Reply to a plan proposal. */
  onPlanReply?: (callId: string, approved: boolean, steps?: PlanStep[], note?: string) => void;
  /** Reply to an ask_user proposal. */
  onAskUserReply?: (callId: string, decision: AskUserDecision) => void;
  /** Custom commands and MCP prompts (`/api/commands`). */
  commands?: CommandInfo[];
};

export type PendingApproval = {
  callId: string;
  call: ToolCall;
  preview: DiffPreview | null;
  /** Parts of a compound command that need approval (Mira's policy). */
  needs?: string[];
};

type Attachment = { path: string; content: string; bytes: number };
/** An image the model will see: base64 without the `data:` prefix. */
export type ImageData = { media_type: string; data: string };

/** Longest edge sent to the model. Bigger screenshots are scaled down —
 *  providers downscale anyway, and it keeps the payload small. */
const IMAGE_MAX_EDGE = 1568;

/** Read an image file, scaled down to IMAGE_MAX_EDGE, as base64. */
async function readImage(file: File): Promise<ImageData> {
  const bitmap = await createImageBitmap(file);
  const scale = Math.min(1, IMAGE_MAX_EDGE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(bitmap.width * scale);
  canvas.height = Math.round(bitmap.height * scale);
  canvas.getContext('2d')!.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  const media_type = file.type === 'image/jpeg' ? 'image/jpeg' : 'image/png';
  const url = canvas.toDataURL(media_type, 0.9);
  return { media_type, data: url.slice(url.indexOf(',') + 1) };
}

/** Cap on how many bytes we'll inline from a single OS-picked file. Larger
 *  files still get a chip in the composer, but the inlined body is
 *  truncated with a marker so a rogue 50 MB video doesn't nuke the model's
 *  context window. Text files usually clock in well under this. */
const NATIVE_ATTACH_MAX_BYTES = 256 * 1024;


export function Composer({
  disabled, busy, mode, model, providerName, cwd,
  environment, environments, envSwitching, onSwitchEnvironment,
  onSend, onSetMode, onSetModel, onSetModelOption, onAcpModes, onAcpCurrentMode, onPickAgentMode, agentDriving, onOpenPicker, onCwdSwitched, onInterrupt, onNewChat, onOpenSettings, onRunReview, onSetGoal, onClearGoal, onCompact, goal, onRemember, onUndo,
  engine, engines, agents, agentsChecking, onCheckAgents, agentConfig, agentDescriptors, onPickProvider, onPickAgent, onConfigureAgents, sessionId, onAgentCompact, onAgentFork, onAgentReverted,
  skills, commands,
  usageRing, pendingApproval, pendingApprovalCount = 0, onAllowAllPending, pendingPlan, pendingAskUser, onDecide, onPlanReply, onAskUserReply,
}: Props) {
  const acpModes_modes = onAcpModes ?? null;
  const acpCurrentMode = onAcpCurrentMode ?? null;
  // The agent's posture spectrum expressed as Mira modes, so the one picker
  // drives either system. Only postures the agent actually has are listed —
  // offering the full five to an agent with three modes would be a lie.
  const agentPicker = useMemo(() => {
    if (!agentDriving || !acpModes_modes || !onPickAgentMode) return null;
    const mapped = mapPosturesToModes(acpModes_modes, acpCurrentMode);
    if (mapped.length === 0) return null;
    const byKey = new Map(mapped.map((o) => [o.posture.key, o]));
    const modes = (Object.keys(MIRA_MODE_TO_POSTURE) as Mode[]).flatMap((m) => {
      const opt = byKey.get(MIRA_MODE_TO_POSTURE[m]);
      return opt ? [{ mode: m as Mode, modeId: opt.modeId }] : [];
    });
    const current = mapped.find((o) => o.current);
    return {
      modes: MODES.filter((d) => modes.some((x) => x.mode === d.value)),
      current: (current ? (Object.keys(MIRA_MODE_TO_POSTURE) as Mode[]).find((m) => MIRA_MODE_TO_POSTURE[m] === current.posture.key) : null) ?? null,
    };
  }, [agentDriving, acpModes_modes, acpCurrentMode, onPickAgentMode]);
  const [text, setText] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [images, setImages] = useState<ImageData[]>([]);
  const [dragging, setDragging] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);
  // Send mode (Settings → General → Composer). When on, plain Enter
  // inserts a newline and only Cmd/Ctrl+Enter sends.
  const [cmdEnterSend] = useBoolPref(PREF_KEYS.composerCmdEnter, false);

  async function addImages(files: File[]) {
    setAttachError(null);
    try {
      const read = await Promise.all(files.map(readImage));
      setImages((prev) => [...prev, ...read]);
    } catch (e) {
      setAttachError(`couldn't read image: ${(e as Error).message}`);
    }
  }
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attachLoading, setAttachLoading] = useState(false);
  const [filePickerOpen, setFilePickerOpen] = useState(false);
  const [slashIdx, setSlashIdx] = useState(0);
  const [slashFeedback, setSlashFeedback] = useState<string | null>(null);
  const [modelPopOpen, setModelPopOpen] = useState(false);
  // Imperative handle on the contenteditable mention input. Powers
  // caret-aware skill-mention insertion (palette picks land at the
  // cursor), programmatic clears (post-send), and palette prefills
  // (`/mode <arg>`). Typing itself flows through the `onChange` prop
  // — the parent never force-writes into the DOM during a keystroke.
  const mentionRef = useRef<MentionInputHandle | null>(null);

  /** Programmatically replace the whole composer content with `next`
   *  (canonical form: plain text + `@skill:<name>` tokens). Delegates
   *  to the MentionInput's imperative API so the DOM chip nodes get
   *  rebuilt; the input's `onChange` handler then re-syncs React state
   *  so callers don't need to `setText(next)` themselves. */
  function updateText(next: string) {
    if (mentionRef.current) {
      mentionRef.current.setText(next);
    } else {
      // Ref not attached yet (shouldn't happen after mount) — fall
      // back to React state so we don't drop the update entirely.
      setText(next);
    }
  }
  // Hidden `<input type="file">` — programmatically clicked by both the
  // `/files` slash command and the `+` menu's "Attach file…" so the OS
  // opens its native file-open dialog. Same list on both paths.
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  function openNativeFiles() {
    // Reset value first so re-picking the same file still fires `change`.
    if (fileInputRef.current) {
      fileInputRef.current.value = '';
      fileInputRef.current.click();
    }
  }
  // "Goal compose" — flipped on by `/goal` (bare). While on, the next
  // Enter fires `onSetGoal(text)` instead of `onSend`. Chip stays until
  // the user either submits the condition or clicks it to abort.
  const [goalComposing, setGoalComposing] = useState(false);
  // Remember the mode we were on before entering plan mode, so toggling the
  // Plan chip off returns you to that mode instead of hardcoding `manual`.
  const [priorMode, setPriorMode] = useState<Mode>(mode === 'plan' ? 'manual' : mode);
  useEffect(() => {
    if (mode !== 'plan') setPriorMode(mode);
  }, [mode]);
  const planActive = mode === 'plan';
  function togglePlan() {
    onSetMode(planActive ? priorMode : 'plan');
  }

  const customCmds = useMemo(() => (commands ?? []).map(customToCommand), [commands]);
  const slash = slashState(text, customCmds);
  const paletteVisible = slash.mode === 'palette';
  // Trigger is `/` OR `@` — `@` promotes `files` to the top of the palette
  // so a bare `@` + Enter fires the OS native file picker without any
  // additional keystrokes. See `filterCommands` for the promotion rule.
  const paletteTrigger = slash.mode === 'palette' || slash.mode === 'args' ? slash.trigger : '/';
  // `/` lists items grouped by where they come from (Mira, your skills,
  // then each plugin and MCP server); `@` keeps its own order so `files`
  // stays first. `paletteMatches` is in display order, so ↑↓ + Enter
  // follow what's on screen.
  const paletteGroups = useMemo(() => {
    if (!paletteVisible) return [];
    const found = filterCommands((slash as { query: string }).query, paletteTrigger, skills, customCmds);
    return paletteTrigger === '@'
      ? [{ key: 'all', label: '', items: found } as PaletteGroup]
      : groupCommands(found);
  }, [paletteVisible, slash, paletteTrigger, skills, customCmds]);
  const paletteMatches = useMemo(() => paletteGroups.flatMap((g) => g.items), [paletteGroups]);

  useEffect(() => { setSlashIdx(0); }, [text]);

  const ctx = useMemo(
    () => ({
      onNewChat,
      onSetMode,
      onSetModel,
      onOpenModelPicker: () => setModelPopOpen(true),
      onOpenFolderPicker: onOpenPicker,
      onOpenNativeFiles: openNativeFiles,
      onOpenSettings,
      onRunReview,
      onSetGoal,
      onClearGoal,
      onCompact,
      onEnterGoalCompose: () => setGoalComposing(true),
      onRemember,
      onUndo,
      // Skill invocation. Insert an `@skill:<name>` chip at the caret
      // rather than firing the message immediately — that used to dump
      // a bare "Use the X skill" with no context, so the model always
      // had to ask "what for?". The token renders as a live chip in
      // the contenteditable input AND in the sent user bubble; the
      // `Skill` tool description teaches the model to treat the token
      // as an invocation directive with the surrounding prose as the
      // argument.
      onInvokeSkill: (name: string) => {
        mentionRef.current?.insertMention(name);
      },
    }),
    [onNewChat, onSetMode, onSetModel, onOpenPicker, onOpenSettings, onRunReview, onSetGoal, onClearGoal, onCompact, onRemember, onUndo, onSend],
  );

  function executeCommand(cmd: SlashCommand, args: string) {
    setSlashFeedback(null);
    try {
      const template = cmd.run(args, ctx);
      if (template === undefined) {
        // Control command — clear the composer.
        updateText('');
      } else {
        // Template — replace composer text so the user can review + send.
        updateText(template);
      }
    } catch (e) {
      // Control command threw a usage/validation error. Keep the text so
      // the user can fix it inline, and surface the error under the
      // composer as an amber hint.
      setSlashFeedback(e instanceof Error ? e.message : String(e));
    }
  }

  function commitPaletteChoice(cmd: SlashCommand) {
    // Prefix = text before the trigger char. Preserves any prose the
    // user wrote before `/gril` so mid-message picks (`hello /gril`)
    // don't nuke the surrounding text.
    const prefix = slash.mode === 'palette' || slash.mode === 'args'
      ? text.slice(0, slash.triggerStart)
      : '';
    if (cmd.isCustom) {
      // Custom command / MCP prompt: with arguments to fill in, prefill
      // `/name `; without, send it now. The server expands it.
      if (cmd.takesArgs || prefix.trim()) {
        updateText(`${prefix}/${cmd.name} `);
      } else {
        onSend(`/${cmd.name}`);
        updateText('');
      }
      return;
    }
    if (cmd.isSkill) {
      // Skill pick — splice out the `/query` and drop in a mention
      // token so it renders as an inline chip. `updateText` mutates
      // the DOM through the mention input's imperative handle, so the
      // chip appears in the composer immediately.
      updateText(`${prefix}@skill:${cmd.name} `);
      return;
    }
    if (cmd.takesArgs) {
      // Prefill `<prefix><trigger><name> ` so the user starts typing
      // arguments; palette auto-hides because there's now a space in
      // the text. Preserves whichever trigger the user typed
      // (`/goal` vs `@goal`).
      updateText(`${prefix}${paletteTrigger}${cmd.name} `);
    } else {
      executeCommand(cmd, '');
    }
  }

  async function attachFile(path: string) {
    setAttachError(null);
    setAttachLoading(true);
    try {
      const f = await readFile(path);
      // De-dupe by path — re-attaching the same file just refreshes it.
      setAttachments((prev) => [
        ...prev.filter((a) => a.path !== f.path),
        { path: f.path, content: f.content, bytes: f.bytes },
      ]);
    } catch (e) {
      setAttachError(String((e as Error).message));
    } finally {
      setAttachLoading(false);
    }
  }

  /** Read one browser `File` (from the OS native picker) into an
   *  Attachment. Text-shaped files inline as-is; binary/oversized files
   *  still get a chip but the inlined content is a short placeholder so
   *  the model at least sees "here's a file called foo.mp4 (14.2 MB)"
   *  even when we can't ship the bytes. */
  async function attachNativeFile(file: File) {
    setAttachError(null);
    setAttachLoading(true);
    try {
      if (/^image\/(png|jpeg|gif|webp)$/.test(file.type)) {
        await addImages([file]);
        return;
      }
      const isBinary = looksBinary(file);
      let content: string;
      if (isBinary || file.size > NATIVE_ATTACH_MAX_BYTES) {
        content = isBinary
          ? `[binary file: ${file.name} (${formatBytes(file.size)}, ${file.type || 'unknown type'}) — content not inlined]`
          : `[oversized file: ${file.name} (${formatBytes(file.size)}) — first ${formatBytes(NATIVE_ATTACH_MAX_BYTES)} inlined]\n\n` +
            (await file.slice(0, NATIVE_ATTACH_MAX_BYTES).text());
      } else {
        content = await file.text();
      }
      setAttachments((prev) => [
        // De-dupe by filename (client-side files have no path).
        ...prev.filter((a) => a.path !== file.name),
        { path: file.name, content, bytes: file.size },
      ]);
    } catch (e) {
      setAttachError(String((e as Error).message));
    } finally {
      setAttachLoading(false);
    }
  }

  async function attachNativeFiles(files: FileList | null) {
    if (!files || files.length === 0) return;
    for (const f of Array.from(files)) {
      await attachNativeFile(f);
    }
  }

  // Out-of-band attachment intake. The whiteboard pane's "Send" produces a
  // PNG in the right panel, far from this component, and threading an
  // imperative handle through a component this size isn't worth it. One
  // event keeps the attachment path — including the binary/image detection
  // and downscaling above — identical to drag-and-drop and paste.
  useEffect(() => {
    function onAttach(e: Event) {
      const file = (e as CustomEvent<File>).detail;
      if (file instanceof File) void attachNativeFile(file);
    }
    window.addEventListener(ATTACH_FILE_EVENT, onAttach);
    return () => window.removeEventListener(ATTACH_FILE_EVENT, onAttach);
  }, []);

  function removeAttachment(path: string) {
    setAttachments((prev) => prev.filter((a) => a.path !== path));
  }

  function submit() {
    const trimmed = text.trim();
    if ((!trimmed && attachments.length === 0 && images.length === 0) || disabled || busy) return;

    // If the user typed a full `/foo bar` command and hit Enter, execute
    // the command instead of sending it as a chat message. Control
    // commands vanish; template commands fill the composer so the user
    // can review — Enter again to actually send.
    if (slash.mode === 'args' && !slash.command.isCustom) {
      executeCommand(slash.command, slash.args);
      return;
    }

    // Goal-compose mode: submit the text as the goal condition instead
    // of a chat message. Clears the compose flag so the next Enter goes
    // back to normal sends.
    if (goalComposing) {
      onSetGoal(trimmed);
      setGoalComposing(false);
      updateText('');
      setSlashFeedback(null);
      return;
    }

    const body = attachments.length > 0 ? renderAttachments(attachments, cwd) + '\n\n' + trimmed : trimmed;
    onSend(body, images.length > 0 ? images : undefined);
    updateText('');
    setAttachments([]);
    setImages([]);
    setAttachError(null);
    setSlashFeedback(null);
  }

  const modeLabel = MODES.find((m) => m.value === mode)?.label ?? mode;

  // Which interactive prompt (if any) is currently waiting for the user.
  // ask_user takes priority (most interactive), then plan, then approval.
  const activePromptKind: 'ask_user' | 'plan' | 'approval' | null =
    pendingAskUser ? 'ask_user'
    : pendingPlan ? 'plan'
    : pendingApproval ? 'approval'
    : null;

  return (
    <div className="flex flex-col items-center gap-1.5 px-4 pb-4 pt-2">
      <form
        className={cn(
          'relative w-full max-w-3xl flex flex-col gap-1.5 rounded-[22px] border border-border bg-secondary/60 p-2.5 transition-colors',
          dragging && 'border-mira-blue/60 bg-mira-blue/[0.06]',
        )}
        onSubmit={(e) => { e.preventDefault(); submit(); }}
        onDragOver={(e) => {
          if (!e.dataTransfer.types.includes('Files')) return;
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDrop={(e) => {
          setDragging(false);
          if (e.dataTransfer.files.length === 0) return;
          e.preventDefault();
          void attachNativeFiles(e.dataTransfer.files);
        }}
      >
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-[22px] text-[13px] font-medium text-mira-blue">
            Drop files or images to attach
          </div>
        )}
        <ImageLightbox src={preview} onClose={() => setPreview(null)} />
        {(planActive || goal || goalComposing) && (
          <div className="flex flex-wrap items-center gap-1.5 px-1.5 pt-0.5">
            {planActive && <PlanChip onExit={togglePlan} />}
            {goalComposing && !goal && (
              <GoalComposeChip onCancel={() => setGoalComposing(false)} />
            )}
            {goal && <GoalChip goal={goal} onClear={onClearGoal} />}
          </div>
        )}

        {!activePromptKind && images.length > 0 && (
          <div className="flex flex-wrap gap-2 px-1.5 pt-1">
            {images.map((img, i) => (
              <div key={i} className="group relative">
                <img
                  src={`data:${img.media_type};base64,${img.data}`}
                  alt=""
                  onClick={() => setPreview(`data:${img.media_type};base64,${img.data}`)}
                  className="size-14 cursor-zoom-in rounded-lg border border-border object-cover"
                />
                <button
                  type="button"
                  aria-label="Remove image"
                  onClick={() => setImages((prev) => prev.filter((_, j) => j !== i))}
                  className="absolute -right-1.5 -top-1.5 flex size-5 items-center justify-center rounded-full border border-border bg-background text-[11px] leading-none text-muted-foreground transition-opacity hover:text-foreground [@media(hover:hover)]:opacity-0 [@media(hover:hover)]:group-hover:opacity-100"
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}

        {!activePromptKind && (attachments.length > 0 || attachError) && (
          <div className="flex flex-wrap gap-1.5 px-1.5">
            {attachments.map((a) => (
              <AttachmentChip key={a.path} attachment={a} onRemove={() => removeAttachment(a.path)} cwd={cwd} />
            ))}
            {attachError && (
              <span className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11.5px] text-destructive">
                {attachError}
              </span>
            )}
          </div>
        )}

        {/* Interactive prompt panel (plan / ask_user / approval) — replaces the
            text input with the relevant dialog, animated from height 0. */}
        <AnimatePresence initial={false}>
          {activePromptKind && (
            <motion.div
              key={activePromptKind}
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: 'auto' }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.2, ease: [0.4, 0, 0.2, 1] }}
              style={{ overflow: 'hidden' }}
            >
              {activePromptKind === 'plan' && pendingPlan && onPlanReply && (
                <EmbeddedPlanCard
                  proposal={pendingPlan.proposal}
                  onApprove={(steps) => onPlanReply(pendingPlan.callId, true, steps)}
                  onCancel={(note) => onPlanReply(pendingPlan.callId, false, undefined, note || undefined)}
                />
              )}
              {activePromptKind === 'ask_user' && pendingAskUser && onAskUserReply && (
                <EmbeddedAskUserCard
                  asker={engine?.kind === 'agent' ? engine.display_name : 'Mira'}
                  proposal={pendingAskUser.proposal}
                  onSubmit={(answers) => onAskUserReply(pendingAskUser.callId, { cancelled: false, answers })}
                  onCancel={() => onAskUserReply(pendingAskUser.callId, { cancelled: true })}
                />
              )}
              {activePromptKind === 'approval' && pendingApproval && onDecide && (
                <EmbeddedApprovalCard
                  approval={pendingApproval}
                  queued={pendingApprovalCount}
                  onAllowAll={onAllowAllPending}
                  onDecide={(allow, scope) => onDecide(pendingApproval.callId, allow, scope)}
                />
              )}
            </motion.div>
          )}
        </AnimatePresence>

        {!activePromptKind && (
          <div className="relative">
            <MentionInput
              handleRef={mentionRef}
              onPasteImages={(files) => void addImages(files)}
              value={text}
              onChange={(next) => { setText(next); setSlashFeedback(null); }}
              onKeyDown={(e) => {
                // Palette is open → arrows navigate, Enter picks, Esc closes.
                if (paletteVisible && paletteMatches.length > 0) {
                  if (e.key === 'ArrowDown') {
                    e.preventDefault();
                    setSlashIdx((i) => Math.min(i + 1, paletteMatches.length - 1));
                    return;
                  }
                  if (e.key === 'ArrowUp') {
                    e.preventDefault();
                    setSlashIdx((i) => Math.max(i - 1, 0));
                    return;
                  }
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    const cmd = paletteMatches[slashIdx];
                    if (cmd) commitPaletteChoice(cmd);
                    return;
                  }
                  if (e.key === 'Tab') {
                    e.preventDefault();
                    const cmd = paletteMatches[slashIdx];
                    if (cmd) {
                      const prefix = slash.mode === 'palette' ? text.slice(0, slash.triggerStart) : '';
                      updateText(`${prefix}${paletteTrigger}${cmd.name}${cmd.takesArgs ? ' ' : ''}`);
                    }
                    return;
                  }
                  if (e.key === 'Escape') {
                    e.preventDefault();
                    // Escape only clears the trigger span, not the whole
                    // composer — mid-message prefix survives.
                    const prefix = slash.mode === 'palette' ? text.slice(0, slash.triggerStart) : '';
                    updateText(prefix);
                    return;
                  }
                }
                // Escape while goal-composing → abort the compose flow
                // without sending anything, matching how Esc dismisses
                // the slash palette above.
                if (goalComposing && e.key === 'Escape') {
                  e.preventDefault();
                  setGoalComposing(false);
                  updateText('');
                  return;
                }
                // Enter behavior depends on the send mode: by default
                // Enter sends and Shift+Enter inserts a newline; with
                // "send with Cmd/Ctrl+Enter" on, plain Enter inserts the
                // newline and only the mod chord sends.
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  submit();
                  return;
                }
                if (e.key === 'Enter' && e.shiftKey) {
                  e.preventDefault();
                  document.execCommand('insertText', false, '\n');
                  return;
                }
                if (e.key === 'Enter' && !cmdEnterSend) { e.preventDefault(); submit(); return; }
                if (e.key === 'Enter' && cmdEnterSend) {
                  e.preventDefault();
                  document.execCommand('insertText', false, '\n');
                  return;
                }
              }}
              placeholder={
                disabled
                  ? 'Waiting for connection…'
                  : goalComposing
                    ? "Describe what 'done' looks like — mira will loop until it's met."
                    : planActive
                      ? 'Describe your task to generate a plan…'
                      : 'Ask mira anything · @ for files · / for commands'
              }
              disabled={disabled}
              roster={skills}
              ariaLabel="Message composer"
            />

            {paletteVisible && paletteMatches.length > 0 && (
              <SlashPalette
                groups={paletteGroups}
                activeIdx={slashIdx}
                onHover={setSlashIdx}
                onPick={commitPaletteChoice}
              />
            )}

            {slash.mode === 'args' && (
              <div className="tooltip pointer-events-none absolute -top-7 left-0">
                {slash.command.origin && (
                  <>
                    <OriginIcon origin={slash.command.origin} fallback={slash.command.icon} />
                    <span className="text-foreground/80">{slash.command.origin.label}</span>
                    <span className="text-muted-foreground/50">·</span>
                  </>
                )}
                <span className="font-mono text-foreground">{slash.trigger}{slash.command.name}</span>
                <span className="truncate">{slash.command.usage.replace(`/${slash.command.name}`, '').trim()}</span>
                {slash.command.kindLabel && (
                  <span className="ml-1 shrink-0 rounded border border-border/60 px-1 text-[10px] uppercase tracking-wide text-muted-foreground/80">
                    {slash.command.kindLabel}
                  </span>
                )}
              </div>
            )}
          </div>
        )}

        {!activePromptKind && slashFeedback && (
          <div className="mx-1 mb-1 rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[11.5px] text-amber-200">
            {slashFeedback}
          </div>
        )}

        {!activePromptKind && (
          <div className="flex items-center gap-1.5 px-1">
            <AttachMenu onAttachFile={openNativeFiles} loading={attachLoading} />

            <SlashButton onClick={() => updateText(text.startsWith('/') || text.startsWith('@') ? text : '/' + text)} />

            <EnginePicker
              open={modelPopOpen}
              onOpenChange={setModelPopOpen}
              engine={engine ?? null}
              fallbackModel={model}
              providerName={providerName ?? null}
              engines={engines}
              agents={agents}
              agentsChecking={agentsChecking}
              onCheckAgents={onCheckAgents}
              agentConfig={agentConfig}
              agentDescriptors={agentDescriptors}
              onPickProvider={(instance, m) => (onPickProvider ? onPickProvider(instance, m) : onSetModel(m, instance))}
              onPickAgent={(driver, m) => onPickAgent?.(driver, m)}
              onSetModelOption={onSetModelOption}
              onConfigureAgents={onConfigureAgents}
              sessionId={sessionId}
              onAgentCompact={onAgentCompact}
              onAgentFork={onAgentFork}
              onAgentReverted={onAgentReverted}
            />

            <ProjectChip cwd={cwd} onClick={onOpenPicker} />

            <span className="flex-1" />

            {agentDriving && agentPicker ? (
              /* One picker for both systems: it lists the agent's postures
                 under Mira's names and confirms the pick in the universal
                 approval dialog. */
              <ModePicker
                mode={agentPicker.current ?? 'manual'}
                label={MODES.find((d) => d.value === agentPicker.current)?.label ?? 'Agent mode'}
                onPick={(m) => onPickAgentMode?.(m)}
                modes={agentPicker.modes.length > 0 ? agentPicker.modes : undefined}
              />
            ) : !agentDriving ? (
              <ModePicker mode={mode} label={modeLabel} onPick={onSetMode} />
            ) : null}
            {/* The agent's own mode lives here, next to Mira's — not inside
                the model picker. Changing what the agent may do without
                asking is a decision, so it opens the universal approval
                dialog rather than switching silently. */}

            {busy ? (
              <button
                type="button"
                onClick={onInterrupt}
                className="flex size-8 items-center justify-center rounded-full bg-mira-error text-[#0e1013] transition-colors hover:brightness-110"
                title="Stop"
                aria-label="Stop"
              >
                <Square className="size-3.5" style={{ fill: 'currentColor' }} />
              </button>
            ) : (
              <button
                type="submit"
                disabled={disabled || (!text.trim() && attachments.length === 0 && images.length === 0)}
                className="flex size-8 items-center justify-center rounded-full bg-foreground text-background transition-opacity hover:opacity-90 disabled:opacity-35"
                title={cmdEnterSend ? 'Send (⌘/Ctrl+Enter)' : 'Send (Enter)'}
                aria-label="Send"
              >
                <ArrowUp className="size-4" />
              </button>
            )}
          </div>
        )}
      </form>

      {/* Footer strip under the composer — usage + worktree only. The
       *  approval pill used to live here; it moved onto the pending
       *  tool card in the transcript so approvals sit next to the
       *  diff/args they act on and don't jump around as the transcript
       *  grows. Global Y/N shortcut is bound at the App level. */}
      <div className="w-full max-w-3xl flex items-center gap-2 px-3">
        <EnvironmentChip
          status={environment ?? null}
          environments={environments ?? []}
          switching={envSwitching ?? null}
          disabled={busy || disabled}
          onSwitch={onSwitchEnvironment}
        />
        <span className="flex-1" />
        {/* Below the composer, not in it: the input row has no room to
            spare in a narrow layout. */}
        {usageRing && <UsageRing data={usageRing} />}
        <WorktreeChip cwd={cwd} onCwdSwitched={onCwdSwitched} />
      </div>

      <FilePicker
        open={filePickerOpen}
        startPath={cwd || undefined}
        onClose={() => setFilePickerOpen(false)}
        onPicked={(p) => { attachFile(p); }}
      />

      {/* Hidden native file input. `openNativeFiles()` above `.click()`s
       *  this; the OS shows its own open dialog. `multiple` matches
       *  Codex's picker. Kept out of the tab order (aria-hidden + no
       *  focus ring) so keyboard users don't stumble into it. */}
      <input
        ref={fileInputRef}
        type="file"
        multiple
        aria-hidden
        tabIndex={-1}
        className="sr-only"
        onChange={(e) => {
          attachNativeFiles(e.target.files);
        }}
      />
    </div>
  );
}

/* ---------- slash palette ---------- */

function SlashPalette({
  groups, activeIdx, onHover, onPick,
}: {
  /** Sections in display order; `activeIdx` counts across all of them. */
  groups: PaletteGroup[];
  activeIdx: number;
  onHover: (i: number) => void;
  onPick: (cmd: SlashCommand) => void;
}) {
  const activeRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: 'nearest' });
  }, [activeIdx]);
  // Headers only help when there's more than one section.
  const showHeaders = groups.length > 1 || groups.some((g) => g.origin);
  let index = -1;

  return (
    <div
      className={cn(
        'absolute bottom-full left-0 right-0 z-10 mb-2 mx-auto max-w-xl overflow-hidden',
        'rounded-2xl border border-white/[0.07] bg-[#1f2024]/95 backdrop-blur-md',
        'shadow-[0_20px_50px_-16px_rgba(0,0,0,0.85)] ring-1 ring-black/40',
        'animate-fade-in',
      )}
      role="listbox"
    >
      <div className="max-h-[24rem] overflow-y-auto py-1.5">
        {groups.map((g, gi) => (
          <div key={g.key} role="group" aria-label={g.label || undefined}>
            {showHeaders && g.label && (
              <div
                className={cn(
                  'flex items-center gap-2 px-4 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground/70',
                  gi > 0 && 'mt-1 border-t border-white/[0.05]',
                )}
              >
                {g.origin && <OriginIcon origin={g.origin} />}
                <span className="normal-case tracking-normal text-[12px] text-foreground/80">{g.label}</span>
                {g.hint && <span className="font-normal normal-case tracking-normal text-muted-foreground/50">{g.hint}</span>}
              </div>
            )}
            {g.items.map((cmd) => {
              index += 1;
              const i = index;
              const Icon = cmd.icon;
              const active = i === activeIdx;
              return (
                <button
                  key={`${g.key}:${cmd.name}`}
                  ref={active ? activeRef : undefined}
                  type="button"
                  onMouseEnter={() => onHover(i)}
                  onClick={() => onPick(cmd)}
                  role="option"
                  aria-selected={active}
                  className={cn(
                    'flex w-full items-baseline gap-3 px-4 py-1.5 text-left transition-colors',
                    active ? 'bg-white/[0.06]' : 'hover:bg-white/[0.035]',
                  )}
                >
                  <Icon
                    className={cn(
                      'size-[15px] shrink-0 self-center transition-colors',
                      active ? 'text-foreground/85' : 'text-foreground/55',
                    )}
                  />
                  <span
                    className={cn(
                      'shrink-0 text-[13.5px] font-medium tracking-tight',
                      active ? 'text-foreground' : 'text-foreground/90',
                    )}
                  >
                    {displayName(cmd)}
                  </span>
                  <span
                    className={cn(
                      'min-w-0 flex-1 truncate text-[13px]',
                      active ? 'text-muted-foreground' : 'text-muted-foreground/70',
                    )}
                  >
                    {cmd.description}
                  </span>
                  {cmd.kindLabel && (
                    <span className="shrink-0 self-center rounded border border-white/[0.08] px-1.5 text-[10px] uppercase tracking-wide text-muted-foreground/70">
                      {cmd.kindLabel}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

/** A plugin or server's icon, falling back to a letter badge. */
function OriginIcon({ origin, fallback }: { origin: Origin; fallback?: SlashCommand['icon'] }) {
  const src = originIconSrc(origin);
  const [failed, setFailed] = useState(false);
  if (src && !failed) {
    return <img src={src} alt="" className="size-3.5 shrink-0 rounded-[3px]" onError={() => setFailed(true)} />;
  }
  if (fallback) {
    const F = fallback;
    return <F className="size-3.5 shrink-0 text-muted-foreground" />;
  }
  return (
    <span className="flex size-3.5 shrink-0 items-center justify-center rounded-[3px] bg-white/10 text-[9px] font-semibold text-foreground/80">
      {origin.label.charAt(0).toUpperCase()}
    </span>
  );
}

/** Human-facing label for a command. Falls back to Title-Case of the
 *  `name` so single-word commands ("new" → "New") don't need a hand-
 *  written label, while multi-word commands ("pull-request") can
 *  override with a proper display string ("Pull request"). */
function displayName(cmd: SlashCommand): string {
  const n = cmd.label ?? cmd.name;
  if (!n) return n;
  return n.charAt(0).toUpperCase() + n.slice(1).replace(/-/g, ' ');
}

/* ---------- attach menu (+) ---------- */

function AttachMenu({
  onAttachFile, loading,
}: { onAttachFile: () => void; loading: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex size-8 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
          title="Attach"
          aria-label="Attach"
        >
          {/* Override the composer's `fill` default — a fill-weight Plus is chunky
           *  and stands out too much next to the softer / model / mode chips.
           *  Regular weight reads as a clean, standard `+`. */}
          {loading ? <Loader className="size-3.5 animate-spin" /> : <Plus className="size-4" />}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-60 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Attach
        </div>
        <div className="flex flex-col">
          <MenuButton
            icon={<Paperclip className="size-3.5" />}
            label="Attach file…"
            hint="Pick a file to inline into the message"
            onClick={() => { setOpen(false); onAttachFile(); }}
          />
          <MenuButton
            icon={<Camera className="size-3.5" />}
            label="Screenshot"
            hint="Needs a vision model"
            disabled
          />
          <MenuButton
            icon={<Link className="size-3.5" />}
            label="From URL"
            hint="Coming soon"
            disabled
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}

function MenuButton({
  icon, label, hint, disabled, onClick,
}: {
  icon: React.ReactNode;
  label: string;
  hint?: string;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        'flex items-start gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors',
        disabled
          ? 'cursor-not-allowed text-muted-foreground/40'
          : 'text-foreground hover:bg-accent/10',
      )}
    >
      <span className={cn('mt-0.5 shrink-0', disabled ? 'text-muted-foreground/40' : 'text-muted-foreground')}>
        {icon}
      </span>
      <div className="flex flex-col min-w-0">
        <span className="text-[13px]">{label}</span>
        {hint && <span className="text-[11.5px] text-muted-foreground/70">{hint}</span>}
      </div>
    </button>
  );
}

/* ---------- attachment chip ---------- */

function AttachmentChip({
  attachment, cwd, onRemove,
}: { attachment: Attachment; cwd: string; onRemove: () => void }) {
  const label = relativeTo(attachment.path, cwd);
  // "Area.mp4" → filename "Area.mp4", subtype badge "MP4". Files without
  // an extension (`Makefile`) fall back to the byte size as the subtype
  // so the two-line chip still fills sensibly.
  const dot = attachment.path.lastIndexOf('.');
  const filename = label;
  const subtype =
    dot > 0 && dot < attachment.path.length - 1
      ? attachment.path.slice(dot + 1).toUpperCase()
      : formatBytes(attachment.bytes);
  return (
    <span
      className={cn(
        // Codex-style chip: rounded card with a padded file-icon square on the
        // left, filename bold above a muted subtype (extension). Overflow-wide
        // filenames truncate — full path lives in the tooltip.
        'group relative inline-flex items-center gap-2.5 rounded-xl border border-border/60 bg-background/70 py-1.5 pl-2 pr-8',
      )}
      title={`${attachment.path} · ${formatBytes(attachment.bytes)}`}
    >
      <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-md border border-border/70 bg-secondary/70 text-muted-foreground">
        <FileIcon className="size-4" />
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="max-w-[18rem] truncate text-[13px] font-semibold text-foreground">
          {filename}
        </span>
        <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground/80">
          {subtype}
        </span>
      </span>
      <button
        type="button"
        onClick={onRemove}
        className="absolute right-1.5 top-1.5 inline-flex size-4 items-center justify-center rounded-full bg-secondary/90 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        aria-label={`Remove ${label}`}
      >
        <X className="size-2.5" />
      </button>
    </span>
  );
}

function relativeTo(abs: string, base: string): string {
  if (base && abs.startsWith(base + '/')) return abs.slice(base.length + 1);
  if (base && abs === base) return '.';
  const parts = abs.split('/');
  if (parts.length <= 3) return abs;
  return '…/' + parts.slice(-2).join('/');
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Inline attachments at the top of the message the model receives. Uses
 *  a stable "## Attached files" header and per-file fenced blocks with the
 *  path as the info string, so the model can trivially cite `path:line` in
 *  its reply. Language guess = extension. */
function renderAttachments(atts: Attachment[], cwd: string): string {
  const parts = atts.map((a) => {
    const rel = relativeTo(a.path, cwd);
    const lang = extToLang(a.path);
    return `### ${rel}\n\`\`\`${lang}\n${a.content}\n\`\`\``;
  });
  return `## Attached files\n\n${parts.join('\n\n')}`;
}

// Parse the "## Attached files" header out of a user message body so the
// transcript can render each file as a Codex-style chip above the bubble
// instead of dumping the raw fenced content into the reader's face. If
// the body doesn't start with the marker, returns the body unchanged and
// an empty `attachments` list.
//
// Hand-walks the string rather than regexing — the header shape is very
// regular and a manual walk sidesteps the edge cases (backtick fencing +
// non-greedy quantifiers) that made an earlier regex-based parser
// mis-slice filenames on multi-attachment messages.
export function parseSentAttachments(body: string): {
  attachments: Array<{ filename: string; subtype: string }>;
  text: string;
} {
  const marker = '## Attached files';
  if (!body.startsWith(marker)) {
    return { attachments: [], text: body };
  }

  // Skip the marker + up to two trailing newlines. Empty-line-before-the-
  // first-header is what `renderAttachments` writes, but be lenient.
  let i = marker.length;
  while (i < body.length && body[i] === '\n') i++;

  const attachments: Array<{ filename: string; subtype: string }> = [];
  while (i < body.length) {
    // Each block must start with "### " (space required). Anything else
    // is the user's own text — bail so it survives into `text` below.
    if (!body.startsWith('### ', i)) break;
    i += 4;

    // Filename runs to the next newline.
    const nl = body.indexOf('\n', i);
    if (nl < 0) break;
    const filename = basename(body.slice(i, nl).trim());
    i = nl + 1;

    // Opening fence line: "```<lang>\n". Fence is exactly three
    // backticks at the start of the line; lang may be empty.
    if (!body.startsWith('```', i)) break;
    const fenceNl = body.indexOf('\n', i);
    if (fenceNl < 0) break;
    i = fenceNl + 1;

    // Content runs until a line that is exactly "```". Scan
    // line-by-line so a `\`\`\`` embedded mid-line (unlikely for us, but
    // possible in inlined source code) can't fake a fence.
    let closed = false;
    while (i < body.length) {
      const eol = body.indexOf('\n', i);
      const line = eol < 0 ? body.slice(i) : body.slice(i, eol);
      const advance = eol < 0 ? body.length : eol + 1;
      if (line === '```') {
        i = advance;
        closed = true;
        break;
      }
      i = advance;
    }
    if (!closed) break;

    // Blank lines between blocks (or before the user text) — swallow.
    while (i < body.length && body[i] === '\n') i++;

    const dot = filename.lastIndexOf('.');
    const subtype =
      dot > 0 && dot < filename.length - 1
        ? filename.slice(dot + 1).toUpperCase()
        : 'FILE';
    attachments.push({ filename, subtype });
  }

  const text = body.slice(i);
  return { attachments, text };
}


/** Read-only rendering of an attachment chip for the transcript. Same
 *  Codex-style visual as the composer chip, minus the X (nothing to
 *  remove on a sent message). */
export function SentAttachmentChip({
  filename, subtype,
}: { filename: string; subtype: string }) {
  return (
    <span
      className="inline-flex items-center gap-2.5 rounded-xl border border-border/60 bg-background/70 py-1.5 pl-2 pr-3"
      title={filename}
    >
      <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-md border border-border/70 bg-secondary/70 text-muted-foreground">
        <FileIcon className="size-4" />
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="max-w-[18rem] truncate text-[13px] font-semibold text-foreground">
          {filename}
        </span>
        <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground/80">
          {subtype}
        </span>
      </span>
    </span>
  );
}

/** Cheap "should we skip reading this as text?" heuristic. Trusts the
 *  browser-supplied MIME type first (image/*, video/*, audio/*, and the
 *  usual binary application/* families), then falls back to an extension
 *  denylist so files with no MIME (Finder-attached mp4, exe, zip) still
 *  land as binary. Anything unrecognised is treated as text — the
 *  inlined content is capped separately so a mis-guess still can't
 *  blow out the message. */
function looksBinary(file: File): boolean {
  const t = (file.type || '').toLowerCase();
  if (t.startsWith('image/') || t.startsWith('video/') || t.startsWith('audio/')) return true;
  if (
    t === 'application/pdf' ||
    t === 'application/zip' ||
    t === 'application/x-tar' ||
    t === 'application/x-gzip' ||
    t === 'application/octet-stream'
  ) return true;
  const name = file.name.toLowerCase();
  const dot = name.lastIndexOf('.');
  if (dot < 0) return false;
  const ext = name.slice(dot + 1);
  const binaryExts = new Set([
    'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'ico', 'heic', 'heif', 'svg',
    'mp4', 'mov', 'mkv', 'webm', 'avi', 'm4v',
    'mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a',
    'pdf', 'zip', 'gz', 'tar', 'tgz', 'bz2', '7z', 'rar',
    'exe', 'dll', 'so', 'dylib', 'bin', 'wasm',
    'ttf', 'otf', 'woff', 'woff2',
    'psd', 'sketch', 'fig',
  ]);
  return binaryExts.has(ext);
}

function extToLang(path: string): string {
  const dot = path.lastIndexOf('.');
  if (dot < 0) return '';
  const ext = path.slice(dot + 1).toLowerCase();
  const map: Record<string, string> = {
    ts: 'ts', tsx: 'tsx', js: 'js', jsx: 'jsx',
    py: 'python', rb: 'ruby', go: 'go', rs: 'rust', java: 'java',
    kt: 'kotlin', swift: 'swift', c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp',
    cs: 'csharp', php: 'php', sh: 'bash', bash: 'bash', zsh: 'bash',
    yml: 'yaml', yaml: 'yaml', toml: 'toml', json: 'json', md: 'md',
    html: 'html', css: 'css', scss: 'scss', sql: 'sql',
  };
  return map[ext] ?? '';
}

/* ---------- mode picker (popover) ---------- */

function ModePicker({
  mode, label, onPick, modes, disabled, disabledTitle,
}: { mode: Mode; label: string; onPick: (m: Mode) => void; modes?: { value: Mode; label: string; desc: string }[]; disabled?: boolean; disabledTitle?: string }) {
  const listed = modes ?? MODES;
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          title={disabled ? disabledTitle : undefined}
          // shrink-0 + whitespace-nowrap prevents this chip from being
          // squeezed when a long branch name pushes the row past the
          // composer width — before, the label would wrap onto two
          // lines ("Ask each time" → "Ask each\ntime") and vertically
          // bloat the whole toolbar.
          className={cn(
            'inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1.5 text-[12.5px] text-muted-foreground transition-colors',
            disabled ? 'cursor-default opacity-80' : 'hover:bg-mira-elev2 hover:text-foreground',
          )}
        >
          <Circle className="size-3 shrink-0" />
          <span>{label}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-64 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Approval
        </div>
        <div className="flex flex-col">
          {listed.map((m) => (
            <button
              key={m.value}
              type="button"
              className={cn(
                'flex flex-col items-start rounded-md px-2.5 py-2 text-left transition-colors',
                'hover:bg-accent/10 hover:text-foreground',
                m.value === mode && 'text-foreground',
              )}
              onClick={() => { onPick(m.value); setOpen(false); }}
            >
              <span className="text-[13px]">{m.label}{m.value === mode && ' ✓'}</span>
              <span className="text-[11.5px] text-muted-foreground">{m.desc}</span>
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}


function basename(p: string): string {
  if (!p) return '';
  const trimmed = p.replace(/\/+$/, '');
  const i = trimmed.lastIndexOf('/');
  return i >= 0 ? trimmed.slice(i + 1) || '/' : trimmed;
}

/* ---------- slash button (opens the palette by seeding "/") ---------- */

/**
 * Custom slash glyph — Phosphor's `*-Slash` icons are all "crossed-out"
 * variants (BellSlash, ChatSlash, …), and a bare `/` character is too
 * thin to sit alongside chunky filled icons in the composer bar.
 *
 * SVG: rounded, italic-slanted forward slash with a thick stroke.
 * Sized like the other 16px composer icons; `currentColor` inherits the
 * button's text colour so the muted → foreground hover transition still
 * works.
 */
function SlashGlyph({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="2.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M15.5 4.5 L8.5 19.5" />
    </svg>
  );
}

function SlashButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title="Slash commands"
      aria-label="Open slash palette"
      className="inline-flex size-8 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
    >
      <SlashGlyph className="size-4" />
    </button>
  );
}

// (shortToolLabel removed with the composer approval footer — the
// inline card renders its own label via ToolCard's `summarize`.)

/* ---------- plan chip (visible only while plan mode is on) ---------- */

/** Active-state indicator + one-click exit. Renders only when plan mode is
 *  on; clicking it drops the user back to their prior mode. Entering plan
 *  mode happens via `/plan` (autocompletes) or the mode picker. */
function PlanChip({ onExit }: { onExit: () => void }) {
  return (
    <button
      type="button"
      onClick={onExit}
      title="Plan mode on — click to exit"
      className="inline-flex items-center gap-1.5 rounded-full bg-mira-blue/15 px-2.5 py-1.5 text-[12.5px] text-mira-blue transition-colors hover:bg-mira-blue/25"
    >
      <Lightbulb className="size-3 shrink-0" />
      <span>Plan</span>
      <span className="text-mira-blue/70">·</span>
    </button>
  );
}

/* ---------- goal compose chip (visible while typing a new goal condition) ---------- */

/** Sibling of [`PlanChip`]: renders while the user is composing the
 *  goal condition (after `/goal`). Uses a dashed border to signal
 *  "empty / awaiting input" so it visually distinguishes from an
 *  active goal. Click cancels the compose flow. */
function GoalComposeChip({ onCancel }: { onCancel: () => void }) {
  return (
    <button
      type="button"
      onClick={onCancel}
      title="Composing a goal — press Enter to set, or click to cancel"
      className="inline-flex items-center gap-1.5 rounded-full border border-dashed border-mira-purple/50 bg-mira-purple/10 px-2.5 py-1.5 text-[12.5px] text-mira-purple transition-colors hover:bg-mira-purple/20"
    >
      <Target className="size-3 shrink-0" />
      <span className="font-medium">Goal</span>
      <span className="text-mira-purple/70">·</span>
      <span className="opacity-90">describe the condition ↵</span>
    </button>
  );
}

/* ---------- goal chip (visible while a `/goal` is set) ---------- */

/** Mirrors [`PlanChip`]'s pattern so autonomy shows up in the same spot,
 *  just tinted purple to visually distinguish "goal-directed" from
 *  "plan-only". Body shows iteration progress + a truncated condition
 *  so the user sees at a glance how many rounds the autonomous loop
 *  has consumed. Click clears the goal (same as `/goal clear`). */
function GoalChip({ goal, onClear }: { goal: Goal; onClear: () => void }) {
  const running = goal.status === 'active';
  const shortCond =
    goal.condition.length > 48
      ? `${goal.condition.slice(0, 48).trim()}…`
      : goal.condition;
  // Non-active statuses keep the chip visible with a tint so the user
  // can see the terminal state at a glance without having to scroll to
  // the GoalPanel; hover still says "click to clear".
  const tone = running
    ? 'bg-mira-purple/15 text-mira-purple hover:bg-mira-purple/25'
    : goal.status === 'met'
      ? 'bg-emerald-500/15 text-emerald-300 hover:bg-emerald-500/25'
      : goal.status === 'needs_user' || goal.status === 'exhausted'
        ? 'bg-amber-500/15 text-amber-300 hover:bg-amber-500/25'
        : goal.status === 'impossible'
          ? 'bg-red-500/15 text-red-300 hover:bg-red-500/25'
          : 'bg-secondary/60 text-muted-foreground hover:bg-secondary';
  const title = running
    ? `Goal running (${goal.iterations}/${goal.max_iterations}) — click to clear`
    : `Goal ${goal.status.replace('_', ' ')} — click to clear`;
  return (
    <button
      type="button"
      onClick={onClear}
      title={title}
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] transition-colors',
        tone,
      )}
    >
      <Target className="size-3 shrink-0" />
      <span className="font-medium">Goal</span>
      <span className="opacity-70">·</span>
      <span className="tabular-nums opacity-90">
        {goal.iterations}/{goal.max_iterations}
      </span>
      <span className="opacity-70">·</span>
      <span className="max-w-[240px] truncate opacity-90">{shortCond}</span>
    </button>
  );
}

/* ---------- session usage readout (tokens + $ cost) ---------- */

/**
 * Footer readout for the running session. Cost is the headline — it's the
 * lever people actually reason about — with token counts as smaller
 * context on the left. Hover reveals a fuller breakdown so power users can
 * still see the raw numbers without them shouting in the chrome.
 *
 * Renders nothing at all while the session is empty, so a fresh chat
 * doesn't lie by showing "$0.00" before the first turn.
 */
/** The limit closest to running out, with the share left (0–1). */

/* ---------- environment chip (local ↔ remote environments) ---------- */

/** This machine, a scratch copy on this machine, or a cloud sandbox. */
function envIcon(backend: string) {
  if (backend === 'local') return Monitor;
  if (backend === 'scratch') return Copy;
  return Cloud;
}

/** Where the session's tools run. Switching to a remote environment
 *  uploads the current worktree (uncommitted edits too); switching back
 *  merges the changes into the same worktree. The worktree stays the
 *  source of truth, so the worktree chip beside this one is unaffected. */
function EnvironmentChip({
  status,
  environments,
  switching,
  disabled,
  onSwitch,
}: {
  status: EnvironmentStatus | null;
  environments: EnvironmentInfo[];
  switching: string | null;
  disabled: boolean;
  onSwitch?: (target: string) => void;
}) {
  const [open, setOpen] = useState(false);
  if (!status || !onSwitch) return null;
  const remote = status.current !== 'local';
  const Icon = envIcon(status.backend);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] transition-colors max-w-[11rem] min-w-0',
            remote ? 'text-mira-blue hover:bg-mira-blue/10' : 'text-muted-foreground hover:bg-mira-elev2 hover:text-foreground',
          )}
          title={
            switching
              ? switching
              : remote
                ? `tools run in ${status.current} (${status.backend})${status.workspace ? ` at ${status.workspace}` : ''}`
                : 'tools run on this machine'
          }
        >
          {switching ? <Loader className="size-3 shrink-0 animate-spin" /> : <Icon className="size-3 shrink-0" />}
          <span className="truncate">{switching ? 'switching…' : (status.current === 'e2b' ? 'Code Sandbox' : status.current)}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-80 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Environment
        </div>
        {switching && (
          <div className="px-2.5 pb-2 text-[12px] text-muted-foreground/80 break-words">{switching}</div>
        )}
        <div className="flex flex-col">
          {environments.map((e) => {
            const current = e.name === status.current;
            const parked = status.parked.includes(e.name);
            const EIcon = envIcon(e.backend);
            return (
              <button
                key={e.name}
                type="button"
                disabled={disabled || !!switching || current}
                onClick={() => { setOpen(false); onSwitch(e.name); }}
                className={cn(
                  'flex items-start gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                  current ? 'text-foreground' : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
                  (disabled || switching) && !current && 'opacity-50',
                )}
              >
                <EIcon className="size-3.5 shrink-0 mt-0.5" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate">{e.name === 'e2b' ? 'Code Sandbox' : e.name}</span>
                    {e.backend !== 'local' && e.backend !== e.name && (
                      <span className="rounded-sm bg-secondary px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">{e.backend === 'e2b' ? 'Code Sandbox' : e.backend}</span>
                    )}
                    {parked && (
                      <span className="rounded-sm bg-mira-blue/15 px-1 text-[9.5px] uppercase tracking-wider text-mira-blue" title="paused — resumes quickly">paused</span>
                    )}
                  </span>
                  {e.description && (
                    <span className="block truncate text-[11px] text-muted-foreground/70">{e.description.replace(/\bE2B\b/g, 'Code Sandbox')}</span>
                  )}
                </span>
                {current && <span className="text-mira-blue text-xs">✓</span>}
              </button>
            );
          })}
        </div>
        <div className="px-2.5 pt-1.5 pb-1 text-[11px] text-muted-foreground/70 border-t border-border/50 mt-1">
          Remote runs use a copy of this worktree; switching back merges the changes in.
          {disabled && !switching && ' Wait for the current turn to finish to switch.'}
        </div>
      </PopoverContent>
    </Popover>
  );
}

/* ---------- worktree chip (branch + dirty + worktree switcher) ---------- */

function WorktreeChip({
  cwd,
  onCwdSwitched,
}: {
  cwd: string;
  onCwdSwitched?: (path: string, sessionId?: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<GitStatusView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [newBranch, setNewBranch] = useState('');

  async function refresh() {
    try {
      const s = await getGitStatus();
      setStatus(s);
      setLoadError(null);
    } catch (e) {
      setLoadError(String((e as Error).message));
    }
  }

  useEffect(() => {
    if (!cwd) { setStatus(null); return; }
    refresh();
  }, [cwd]);

  async function switchTo(path: string) {
    setBusy(true);
    try {
      const { session_id } = await putCwd(path);
      // Hand the new slot id up to the parent so it can attach its WS.
      // Without this, the socket keeps forwarding the old slot's frames
      // and the transcript silently stays on the previous folder.
      onCwdSwitched?.(path, session_id);
      // Also refresh git state so the chip label reflects the new
      // worktree before the WS attach lands.
      await refresh();
      setOpen(false);
    } catch (e) {
      setLoadError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  async function createAndSwitch() {
    const branch = newBranch.trim();
    if (!branch) return;
    setBusy(true);
    try {
      const wt = await createWorktree(branch);
      setNewBranch('');
      await switchTo(wt.path);
    } catch (e) {
      setLoadError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  /** Click-through for an existing (or remote-only) branch in the
   *  "Switch to branch" list. Reuses the same `createWorktree` →
   *  `switchTo` path as the "New worktree" input; the backend picks
   *  between checkout-existing, create-tracking, and create-fresh. */
  async function createAndSwitchTo(branch: string) {
    setBusy(true);
    try {
      const wt = await createWorktree(branch);
      await switchTo(wt.path);
    } catch (e) {
      setLoadError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  }

  const label = status?.in_repo
    ? (status.branch ?? 'detached')
    : 'no git';
  const primary = status?.worktrees.find((w) => !isLinkedWorktree(w.path, status.worktrees));

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] text-muted-foreground hover:bg-mira-elev2 hover:text-foreground transition-colors max-w-[10rem] min-w-0"
          title={status?.in_repo ? `branch: ${label}${status.dirty ? ' (dirty)' : ''}` : 'not a git repo'}
        >
          <GitBranch className="size-3 shrink-0" />
          <span className="truncate">{label}</span>
          {status?.dirty && <span className="size-1.5 rounded-full bg-amber-500 shrink-0" title="uncommitted changes" />}
          {status?.is_worktree && (
            <span className="rounded-sm bg-mira-blue/15 px-1 text-[9.5px] uppercase tracking-wider text-mira-blue">wt</span>
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-72 p-1.5" align="start">
        <div className="px-2 pb-1.5 pt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          Working tree
        </div>
        {!status?.in_repo && (
          <div className="px-2.5 py-2 text-[12px] text-muted-foreground/70">Not a git repo.</div>
        )}
        {status?.in_repo && (
          <>
            <div className="flex flex-col">
              {status.worktrees.map((w) => (
                <button
                  key={w.path}
                  type="button"
                  disabled={busy || w.is_current}
                  onClick={() => switchTo(w.path)}
                  className={cn(
                    'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                    w.is_current ? 'text-foreground' : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
                    busy && !w.is_current && 'opacity-50',
                  )}
                >
                  <GitBranch className="size-3.5 shrink-0" />
                  <span className="min-w-0 flex-1 truncate">{w.branch ?? 'detached'}</span>
                  {w.is_current && <span className="text-mira-blue text-xs">✓</span>}
                  {primary && w.path === primary.path && (
                    <span className="rounded-sm bg-secondary px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">main</span>
                  )}
                </button>
              ))}
            </div>
            {(() => {
              // Existing branches not already checked out in a worktree.
              // Clicking creates `.mira/worktrees/<branch>` (or a local
              // tracking branch on top of the remote) and switches to it
              // via the same `createWorktree` → `switchTo` path the
              // "New worktree" input takes.
              const available = (status.branches ?? []).filter((b) => !b.in_worktree);
              if (available.length === 0) return null;
              return (
                <div className="mt-1.5 border-t border-border/60 pt-1.5">
                  <div className="px-2.5 pb-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Switch to branch
                  </div>
                  <div className="max-h-56 overflow-y-auto flex flex-col">
                    {available.map((b) => (
                      <button
                        key={`${b.is_remote ? 'r' : 'l'}:${b.name}`}
                        type="button"
                        disabled={busy}
                        onClick={() => createAndSwitchTo(b.name)}
                        className={cn(
                          'flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
                          'text-muted-foreground hover:bg-accent/40 hover:text-foreground',
                          busy && 'opacity-50',
                        )}
                        title={b.upstream ? `tracks ${b.upstream}` : b.name}
                      >
                        <GitBranch className="size-3.5 shrink-0 opacity-60" />
                        <span className="min-w-0 flex-1 truncate">{b.name}</span>
                        {b.is_remote && (
                          <span className="rounded-sm bg-secondary/60 px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">
                            remote
                          </span>
                        )}
                      </button>
                    ))}
                  </div>
                </div>
              );
            })()}
            <div className="mt-1.5 border-t border-border/60 pt-1.5">
              <div className="px-2.5 pb-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
                New worktree
              </div>
              <div className="flex items-center gap-1.5 px-1.5">
                <input
                  value={newBranch}
                  onChange={(e) => setNewBranch(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); createAndSwitch(); } }}
                  placeholder="branch name"
                  disabled={busy}
                  className="flex-1 min-w-0 rounded-md border border-border bg-background/60 px-2 py-1 text-[12.5px] outline-none focus:border-mira-blue/60"
                />
                <button
                  type="button"
                  onClick={createAndSwitch}
                  disabled={busy || !newBranch.trim()}
                  className="rounded-md bg-foreground px-2.5 py-1 text-[12px] text-background transition-opacity hover:opacity-90 disabled:opacity-35"
                >
                  Add
                </button>
              </div>
            </div>
          </>
        )}
        {loadError && (
          <div className="mx-1.5 mt-1.5 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] text-destructive">
            {loadError}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}

/** A worktree is "linked" (not the primary) when it lives under any other
 *  worktree's `.git/worktrees/<name>`. We approximate cheaply: if the path
 *  is *inside* one of the other entries' paths, treat it as linked. Good
 *  enough for display sorting; the backend already tags `is_current`. */
function isLinkedWorktree(path: string, all: { path: string }[]): boolean {
  return all.some((o) => o.path !== path && path.startsWith(o.path + '/'));
}

/* ---------- embedded prompt components (plan / ask_user / approval) ---------- */

/** Plan proposal rendered directly inside the Composer (no outer card
 *  border — the Composer form provides the container). */
function EmbeddedPlanCard({
  proposal,
  onApprove,
  onCancel,
}: {
  proposal: PlanProposal;
  onApprove: (steps: PlanStep[]) => void;
  onCancel: (note: string) => void;
}) {
  const [steps, setSteps] = useState<PlanStep[]>(() =>
    proposal.steps.map((s) => ({ description: s.description, why: s.why ?? undefined })),
  );
  const [note, setNote] = useState('');
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    setSteps(proposal.steps.map((s) => ({ description: s.description, why: s.why ?? undefined })));
    setDirty(false);
  }, [proposal]);

  function updateStep(i: number, patch: Partial<PlanStep>) {
    setSteps((prev) => prev.map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
    setDirty(true);
  }
  function move(i: number, dir: -1 | 1) {
    const j = i + dir;
    if (j < 0 || j >= steps.length) return;
    setSteps((prev) => {
      const next = [...prev];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
    setDirty(true);
  }
  function remove(i: number) {
    setSteps((prev) => prev.filter((_, idx) => idx !== i));
    setDirty(true);
  }
  function add() {
    setSteps((prev) => [...prev, { description: '' }]);
    setDirty(true);
  }

  const canApprove = steps.length > 0 && steps.every((s) => s.description.trim().length > 0);

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-1.5 pt-1 pb-2">
        <PhLightbulb fill="currentColor" className="size-3.5 shrink-0 text-mira-blue" />
        <div className="min-w-0 flex-1">
          <div className="text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground/80">
            Proposed plan
          </div>
          <div className="truncate text-[13.5px] font-semibold text-foreground">
            {proposal.title}
          </div>
        </div>
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
          {steps.length} step{steps.length === 1 ? '' : 's'}
        </span>
      </div>

      <div className="border-t border-border/30" />

      <div className="max-h-[38vh] overflow-y-auto py-2">
        <div className="flex flex-col gap-1 px-1.5">
          {steps.map((s, i) => (
            <EmbeddedStepRow
              key={i}
              index={i}
              step={s}
              onChange={(patch) => updateStep(i, patch)}
              onMoveUp={i === 0 ? undefined : () => move(i, -1)}
              onMoveDown={i === steps.length - 1 ? undefined : () => move(i, 1)}
              onRemove={steps.length === 1 ? undefined : () => remove(i)}
            />
          ))}
        </div>
        <button
          type="button"
          onClick={add}
          className="mx-1.5 mt-1.5 inline-flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          <Plus className="size-3" /> Add step
        </button>
      </div>

      <div className="border-t border-border/30" />

      <div className="flex flex-col gap-2 px-1.5 py-2.5">
        <input
          type="text"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="Optional note (shown if you cancel)"
          className="w-full rounded-md bg-secondary/50 px-2.5 py-1.5 text-[12px] outline-none placeholder:text-muted-foreground/50 focus:bg-secondary/70"
        />
        <div className="flex items-center justify-end gap-1.5">
          <span className="mr-auto text-[11px] text-muted-foreground/80">
            {dirty ? 'Approve will run the edited plan' : '⌘↵ to approve'}
          </span>
          <button
            type="button"
            onClick={() => onCancel(note)}
            className="rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onApprove(steps)}
            disabled={!canApprove}
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold transition-all',
              canApprove
                ? 'bg-foreground text-background hover:brightness-95'
                : 'cursor-not-allowed bg-secondary/60 text-muted-foreground',
            )}
          >
            {dirty ? 'Approve with edits' : 'Approve'}
            <PhArrowRight className="size-3" strokeWidth={2.5} />
          </button>
        </div>
      </div>
    </div>
  );
}

function EmbeddedStepRow({
  index, step, onChange, onMoveUp, onMoveDown, onRemove,
}: {
  index: number;
  step: PlanStep;
  onChange: (patch: Partial<PlanStep>) => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  onRemove?: () => void;
}) {
  return (
    <div className="group flex items-start gap-2.5 rounded-xl bg-secondary/40 px-3 py-2 transition-colors hover:bg-secondary/60">
      <span className="mt-[3px] inline-flex size-5 shrink-0 items-center justify-center rounded-full bg-background/60 text-[10.5px] font-semibold text-muted-foreground ring-1 ring-inset ring-border">
        {index + 1}
      </span>
      <div className="min-w-0 flex-1">
        <input
          type="text"
          value={step.description}
          onChange={(e) => onChange({ description: e.target.value })}
          placeholder="What this step does"
          className="w-full bg-transparent text-[13px] outline-none placeholder:text-muted-foreground/50"
        />
        {(step.why != null || step.description) && (
          <input
            type="text"
            value={step.why ?? ''}
            onChange={(e) => onChange({ why: e.target.value || undefined })}
            placeholder="Why (optional)"
            className="mt-0.5 w-full bg-transparent text-[11.5px] text-muted-foreground outline-none placeholder:text-muted-foreground/40"
          />
        )}
      </div>
      <div className="flex items-center opacity-0 transition-opacity group-hover:opacity-100">
        <EmbeddedIconBtn onClick={onMoveUp} disabled={!onMoveUp} title="Move up">
          <PhArrowUp className="size-3" />
        </EmbeddedIconBtn>
        <EmbeddedIconBtn onClick={onMoveDown} disabled={!onMoveDown} title="Move down">
          <PhArrowDown className="size-3" />
        </EmbeddedIconBtn>
        <EmbeddedIconBtn onClick={onRemove} disabled={!onRemove} title="Remove">
          <Trash className="size-3" />
        </EmbeddedIconBtn>
      </div>
    </div>
  );
}

function EmbeddedIconBtn({
  onClick, disabled, title, children,
}: {
  onClick?: () => void;
  disabled?: boolean;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="rounded p-1 text-muted-foreground/70 transition-colors hover:bg-background/60 hover:text-foreground disabled:opacity-30 disabled:hover:bg-transparent"
    >
      {children}
    </button>
  );
}

/** ask_user proposal rendered directly inside the Composer. */
function EmbeddedAskUserCard({
  proposal,
  onSubmit,
  onCancel,
  asker = 'Mira',
}: {
  proposal: AskUserProposal;
  /** Who is asking — Mira, or the agent driving this chat. */
  asker?: string;
  onSubmit: (answers: AskUserAnswer[]) => void;
  onCancel: () => void;
}) {
  type Draft = { picked: string[]; custom: string | undefined };
  function emptyDraft(): Draft { return { picked: [], custom: undefined }; }

  const [drafts, setDrafts] = useState<Draft[]>(() => proposal.questions.map(emptyDraft));
  const [idx, setIdx] = useState(0);
  const [dir, setDir] = useState<1 | -1>(1);
  const customRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    setDrafts(proposal.questions.map(emptyDraft));
    setIdx(0);
    setDir(1);
  }, [proposal]);

  const total = proposal.questions.length;
  const clampedIdx = Math.min(idx, total - 1);
  const isLast = clampedIdx === total - 1;
  const currentDraft = drafts[clampedIdx] ?? emptyDraft();
  const currentReady =
    currentDraft.picked.length > 0 ||
    (currentDraft.custom ?? '').trim().length > 0;

  function updateDraft(i: number, patch: (d: Draft) => Draft) {
    setDrafts((prev) => prev.map((d, j) => (j === i ? patch(d) : d)));
  }

  function advance() {
    if (!currentReady) return;
    if (!isLast) {
      setDir(1);
      setIdx((i) => Math.min(i + 1, total - 1));
      return;
    }
    const answers: AskUserAnswer[] = drafts.map((d) => ({
      picked: d.picked,
      custom: (d.custom ?? '').trim() ? d.custom!.trim() : null,
    }));
    onSubmit(answers);
  }

  const q = proposal.questions[clampedIdx];
  const multi = q.multi_select === true;
  const customOpen = currentDraft.custom !== undefined;

  function toggleOption(label: string) {
    updateDraft(clampedIdx, (d) => {
      const next: Draft = { ...d, custom: undefined };
      if (multi) {
        next.picked = d.picked.includes(label)
          ? d.picked.filter((l) => l !== label)
          : [...d.picked, label];
      } else {
        next.picked = d.picked.includes(label) ? [] : [label];
      }
      return next;
    });
  }

  function openCustom() {
    updateDraft(clampedIdx, (d) => ({ picked: [], custom: d?.custom ?? '' }));
    requestAnimationFrame(() => customRef.current?.focus());
  }
  function closeCustom() {
    updateDraft(clampedIdx, (d) => ({ ...d, custom: undefined }));
  }

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2.5 px-1.5 pt-1 pb-2">
        <Sparkle fill="currentColor" className="size-3.5 text-mira-blue" />
        <span className="text-[12.5px] font-semibold tracking-tight text-foreground">
          {asker} needs your input
        </span>
        <span className="ml-auto text-[11px] tabular-nums text-muted-foreground">
          {clampedIdx + 1} of {total}
        </span>
      </div>

      <div className="border-t border-border/30" />

      <AnimatePresence initial={false} mode="wait">
      <motion.div
        key={clampedIdx}
        initial={{ opacity: 0, x: dir * 24 }}
        animate={{ opacity: 1, x: 0 }}
        exit={{ opacity: 0, x: dir * -24 }}
        transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
        className="flex flex-col gap-2.5 px-1.5 py-3 max-h-[45vh] overflow-y-auto"
      >
        {q.header && (
          <div className="text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground/80">
            {q.header}
          </div>
        )}
        <div className="text-[13.5px] font-medium leading-snug text-foreground">
          {q.question}
        </div>
        <div className="flex flex-col gap-1.5">
          {q.options.map((opt) => {
            const active = currentDraft.picked.includes(opt.label);
            return (
              <button
                key={opt.label}
                type="button"
                onClick={() => toggleOption(opt.label)}
                className={cn(
                  'group flex items-start gap-3 rounded-xl px-3 py-2.5 text-left transition-colors',
                  active ? 'bg-white text-black' : 'bg-secondary/40 hover:bg-secondary/60',
                )}
              >
                <span
                  className={cn(
                    'mt-[3px] flex size-[14px] shrink-0 items-center justify-center transition-colors',
                    multi ? 'rounded-[5px]' : 'rounded-full',
                    active ? 'bg-black' : 'bg-background/60 ring-1 ring-inset ring-border',
                  )}
                >
                  {active && multi && <PhCheck className="size-2.5 text-white" strokeWidth={2.5} />}
                  {active && !multi && <span className="size-1.5 rounded-full bg-white" />}
                </span>
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <div className="flex items-center gap-1.5">
                    <span className={cn('text-[13px] font-medium', active ? 'text-black' : 'text-foreground')}>
                      {opt.label}
                    </span>
                    {opt.recommended && (
                      <span className={cn('text-[10px] font-medium uppercase tracking-wider', active ? 'text-black/60' : 'text-muted-foreground/80')}>
                        · Recommended
                      </span>
                    )}
                  </div>
                  {opt.description && (
                    <span className={cn('text-[11.5px] leading-snug', active ? 'text-black/70' : 'text-muted-foreground')}>
                      {opt.description}
                    </span>
                  )}
                </div>
              </button>
            );
          })}
          {!customOpen && (
            <button
              type="button"
              onClick={openCustom}
              className="group flex items-center gap-2.5 rounded-xl px-3 py-2 text-left text-[12px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground"
            >
              <PenLine className="size-3.5" />
              <span>Something else</span>
            </button>
          )}
          {customOpen && (
            <div className="rounded-xl bg-secondary/60 p-2.5">
              <div className="flex items-center gap-1.5 pb-1.5 text-[10px] font-semibold uppercase tracking-[0.11em] text-muted-foreground">
                <PenLine className="size-3" />
                <span>Free response</span>
                <button
                  type="button"
                  onClick={closeCustom}
                  className="ml-auto rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-secondary hover:text-foreground"
                  aria-label="Close free-text"
                >
                  <X className="size-3" />
                </button>
              </div>
              <textarea
                ref={customRef}
                value={currentDraft.custom ?? ''}
                onChange={(e) =>
                  updateDraft(clampedIdx, () => ({ picked: [], custom: e.target.value }))
                }
                rows={2}
                placeholder="Do it a different way — describe what you want instead"
                className="min-h-[3rem] w-full resize-y border-0 bg-transparent p-0 text-[12.5px] leading-snug text-foreground outline-none placeholder:text-muted-foreground/50"
              />
            </div>
          )}
        </div>
      </motion.div>
      </AnimatePresence>

      <div className="border-t border-border/30" />

      <div className="flex items-center gap-1.5 px-1.5 py-2.5">
        {clampedIdx > 0 ? (
          <button
            type="button"
            onClick={() => { setDir(-1); setIdx((i) => Math.max(0, i - 1)); }}
            className="inline-flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            <PhArrowLeft className="size-3" strokeWidth={2.5} />
            Back
          </button>
        ) : (
          <span />
        )}
        <button
          type="button"
          onClick={onCancel}
          className="ml-auto rounded-md px-2.5 py-1.5 text-[11.5px] font-medium text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          Skip
        </button>
        <button
          type="button"
          onClick={advance}
          disabled={!currentReady}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold transition-all',
            currentReady
              ? 'bg-foreground text-background hover:brightness-95'
              : 'cursor-not-allowed bg-secondary/60 text-muted-foreground',
          )}
        >
          {isLast ? 'Send answers' : 'Next'}
          <PhArrowRight className="size-3" strokeWidth={2.5} />
        </button>
      </div>
    </div>
  );
}

/** Tool approval rendered directly inside the Composer. */
function EmbeddedApprovalCard({
  approval,
  onDecide,
  queued = 1,
  onAllowAll,
}: {
  approval: PendingApproval;
  onDecide: (allow: boolean, scope?: ApprovalScope) => void;
  /** Requests waiting, this one included. */
  queued?: number;
  onAllowAll?: () => void;
}) {
  const { call, preview } = approval;
  // One card for every approval. An external agent's request only differs
  // in how it is described: its own tool name and input, and why it asks.
  const agent = useMemo(() => agentRequestOf(call), [call]);
  const info = infoFor(agent ? agentToolAsMira(agent.tool) : call.function.name);
  const Icon = info.Icon;
  const isDiffTool = call.function.name === 'write_file' || call.function.name === 'edit_file';
  const prettyArgs = useMemo(() => {
    if (agent) {
      const headline = agentRequestHeadline(agent);
      return headline ?? JSON.stringify(agent.input, null, 2);
    }
    try { return JSON.stringify(JSON.parse(call.function.arguments), null, 2); }
    catch { return call.function.arguments; }
  }, [call.function.arguments, agent]);
  // A compound shell command, as the operations it is made of — one long
  // line hides which part is the one worth a second look.
  const command = useMemo(() => {
    if (agent) return agent.tool === 'Bash' && typeof agent.input.command === 'string' ? agent.input.command : null;
    if (call.function.name !== 'bash') return null;
    try {
      const c = JSON.parse(call.function.arguments)?.command;
      return typeof c === 'string' ? c : null;
    } catch {
      return null;
    }
  }, [agent, call]);
  const parts = useMemo(() => (command ? splitShellCommand(command) : []), [command]);
  // Which parts are the reason for asking: the agent says so in its
  // reason; for Mira's own commands the server's policy names them.
  const flagged = useMemo(
    () => (agent ? partsNeedingApproval(agent.reason) : (approval.needs ?? [])),
    [agent, approval.needs],
  );

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2 px-1.5 pt-1 pb-2 font-mono text-[12.5px]">
        <span className="shrink-0 text-mira-tool"><Icon className="size-3.5" /></span>
        <span className="font-medium text-foreground truncate min-w-0">
          {agent ? agent.tool : info.verbCont}{' '}
          <span className="font-normal text-muted-foreground">
            {isDiffTool && preview?.path
              ? preview.path.split('/').slice(-2).join('/')
              : ''}
          </span>
        </span>
        {preview && (
          <span className="rounded-full border border-border bg-background px-1.5 py-0.5 text-[10.5px] uppercase tracking-wider text-muted-foreground shrink-0">
            {preview.kind}
          </span>
        )}
        <span className="ml-auto shrink-0 rounded-full bg-secondary px-2 py-0.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground">
          {queued > 1 ? `1 of ${queued} awaiting` : 'awaiting approval'}
        </span>
      </div>

      <div className="max-h-[35vh] overflow-auto">
        {preview ? (
          <div className="diff">
            {preview.lines.map((line: DiffLine, i: number) => {
              if (line.tag === 'hunkgap') return <div key={i} className="diff-line hunk">···</div>;
              const cls = line.tag === 'add' ? 'add' : line.tag === 'del' ? 'del' : 'ctx';
              const prefix = line.tag === 'add' ? '+' : line.tag === 'del' ? '-' : ' ';
              return (
                <div key={i} className={`diff-line ${cls}`}>
                  <span className="prefix">{prefix}</span>
                  <span className="text">{line.text}</span>
                </div>
              );
            })}
          </div>
        ) : parts.length > 1 ? (
          <ol className="m-0 max-h-[22vh] list-none space-y-0.5 overflow-auto rounded-md bg-background/60 px-2 py-1.5 font-mono text-xs">
            {parts.map((p, i) => {
              const hot = flagged.some((f) => f.includes(p.text) || p.text.includes(f));
              return (
                <li key={i} className="flex items-start gap-2">
                  <span className="w-4 shrink-0 select-none text-right text-muted-foreground/45">{i + 1}</span>
                  <span className={cn('min-w-0 flex-1 whitespace-pre-wrap break-all', hot ? 'text-foreground' : 'text-muted-foreground')}>
                    {p.text}
                    {p.joiner && <span className="ml-1.5 text-muted-foreground/40">{p.joiner}</span>}
                  </span>
                  {hot && <span className="mt-1 size-1.5 shrink-0 rounded-full bg-mira-warn" title="Needs approval" />}
                </li>
              );
            })}
          </ol>
        ) : (
          <pre className="m-0 max-h-[22vh] overflow-auto whitespace-pre-wrap rounded-md bg-background/60 px-3 py-2 font-mono text-xs text-muted-foreground">
            {prettyArgs}
          </pre>
        )}
      </div>

      {flagged.length > 0 ? (
        <div className="px-1.5 pt-2 text-[11.5px] text-muted-foreground">
          <div className="text-foreground/75">
            {flagged.length === 1 ? 'This part needs approval:' : `These ${flagged.length} parts need approval:`}
          </div>
          <ul className="mt-1 max-h-24 space-y-0.5 overflow-auto font-mono text-[11px]">
            {flagged.map((f, i) => (
              <li key={i} className="flex gap-1.5">
                <span className="mt-[5px] size-1.5 shrink-0 rounded-full bg-mira-warn" />
                <span className="min-w-0 break-all">{f}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : agent?.reason ? (
        <div className="px-1.5 pt-2 text-[11.5px] text-muted-foreground">
          <span className="text-foreground/75">Why it asks:</span> {agent.reason}
        </div>
      ) : null}

      <div className="border-t border-border/30" />

      {/* One shared row of decisions. The composer's inline card and the
          approval modal are the same question asked in two places, and they
          were rendering their own buttons — which is how "Allow" ends up
          meaning slightly different things depending on which appeared. */}
      <div className="px-1.5 py-2">
        <ApprovalChoices
          choices={[
            ...(queued > 1 && onAllowAll
              ? [{ id: 'all', label: `Allow all ${queued}`, title: 'Allow every waiting request once' }]
              : []),
            { id: 'deny', label: 'Deny', title: 'Deny (n)' },
            { id: 'always', label: 'Always allow', title: 'Never ask again' },
            { id: 'session', label: 'Allow for this chat', title: 'Stop asking about this until the chat ends' },
            { id: 'once', label: 'Allow', primary: true, title: 'Allow this one call (y)' },
          ]}
          onChoose={(id: string) => {
            if (id === 'all') onAllowAll?.();
            else if (id === 'deny') onDecide(false);
            else onDecide(true, id === 'always' ? 'always' : id === 'session' ? 'session' : 'once');
          }}
        />
      </div>
      <div className="px-1.5 pb-1.5 text-right text-[10.5px] text-muted-foreground/60">
        <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">y</kbd> allow ·{' '}
        <kbd className="rounded bg-secondary/70 px-1 py-0.5 font-mono text-[10px]">n</kbd> deny
      </div>
    </div>
  );
}

/* ---------- project chip (opens folder picker) ---------- */

function ProjectChip({ cwd, onClick }: { cwd: string; onClick: () => void }) {
  // Prefer the primary-worktree name when the cwd is a linked worktree so
  // switching branches doesn't visually change the project. Falls back to
  // the cwd basename in every other case (no git, load error, primary tree).
  const [primary, setPrimary] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    if (!cwd) { setPrimary(null); return; }
    getGitStatus()
      .then((s) => {
        if (cancelled) return;
        setPrimary(s.is_worktree ? (s.primary_project ?? null) : null);
      })
      .catch(() => { if (!cancelled) setPrimary(null); });
    return () => { cancelled = true; };
  }, [cwd]);

  const fallback = cwd ? basename(cwd) : 'Choose project';
  const label = primary ?? fallback;
  const title = primary
    ? `${primary} (worktree at ${cwd})`
    : (cwd || 'Choose a folder');
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1.5 text-[12.5px] text-muted-foreground hover:bg-mira-elev2 hover:text-foreground transition-colors max-w-[9rem] min-w-0"
    >
      <Folder className="size-3 shrink-0" />
      <span className="truncate">{label}</span>
    </button>
  );
}

/** An agent's tool name as the Mira tool it corresponds to, for the icon
 *  and verb. Unknown tools keep their own name. */
function agentToolAsMira(tool: string): string {
  const map: Record<string, string> = {
    Bash: 'bash', Read: 'read_file', Write: 'write_file', Edit: 'edit_file', MultiEdit: 'edit_file',
    Glob: 'glob', Grep: 'grep', WebFetch: 'web_fetch', WebSearch: 'web_search',
  };
  return map[tool] ?? tool;
}
