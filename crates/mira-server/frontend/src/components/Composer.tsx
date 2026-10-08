import { ComposerNoticeStack, type ComposerNotice } from './ComposerNoticeStack';
import { captureScreenshot } from '../lib/screenCapture';
import { ImageAttachmentDetails } from './ImageAttachmentDetails';
import { reportWorkspaceSetup, type WorkspaceSetup } from './WorkspaceSetupCard';
import { useEffect, useLayoutEffect, useId, useMemo, useRef, useState } from 'react';
import { ImageLightbox } from './ImageLightbox';
import { AnimatePresence, m, useReducedMotion } from 'framer-motion';
import {
  ArrowUp,
  Camera,
  Cloud,
  Copy,
  CornerDownRight,
  File as FileIcon,
  Folder,
  GitBranch,
  Lightbulb,
  Link,
  Loader,
  Monitor,
  MoreHorizontal,
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
import { ApprovalChoices, ApprovalModeIcon } from './ApprovalDialog';
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
import { useIsPhone } from '../lib/mobile';
import { draftKey } from '../lib/composerDrafts';
import { cachedDraft, changeRichDraft, clearRichDraft, restoreRichDraft, type RichDraft } from '../lib/richComposerDrafts';
import { ATTACH_FILE_EVENT, COMPOSE_QUOTE_EVENT, COMPOSE_TEXT_EVENT, registerAttachmentIntake, type ComposerQuote } from '@/lib/attachBridge';
import { QuoteCard, quoteMarkdown } from './QuoteCard';
import { shortcutLabelForCommand, useKeybindings } from '@/lib/keybindings';
import { Dialog, DialogContent, DialogTitle, DialogDescription } from './ui/dialog';
import { ChevronDown, GripVertical } from 'lucide-react';

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
  onQueueMessage?: (text: string, images?: ImageData[]) => void;
  queuedMessages?: QueuedComposerMessage[];
  onRemoveQueuedMessage?: (id: string) => void;
  onEditQueuedMessage?: (item: QueuedComposerMessage, text: string) => Promise<void>;
  onReorderQueuedMessage?: (id: string, beforeId: string | null) => Promise<void>;
  onSteerQueuedMessage?: (id: string) => void;
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
  pendingApprovals?: PendingApproval[];
  ruleEditorCallId?: string | null;
  onRuleEditorCancel?: () => void;
  onActiveApprovalChange?: (callId: string | null) => void;
  pendingPlans?: { callId: string; proposal: PlanProposal }[];
  pendingQuestions?: { callId: string; proposal: AskUserProposal }[];
  notices?: ComposerNotice[];
  /** How many approvals are waiting, including the one shown. */
  pendingApprovalCount?: number;
  /** Allow every waiting request once. */
  onAllowAllPending?: () => void;
  /** Active plan proposal waiting for the user to approve/cancel. */
  pendingPlan?: { callId: string; proposal: PlanProposal } | null;
  /** Active ask_user proposal waiting for answers. */
  pendingAskUser?: { callId: string; proposal: AskUserProposal } | null;
  /** Approve/deny a pending tool call. */
  onDecide?: (callId: string, allow: boolean, scope?: ApprovalScope, rules?: string[]) => void;
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
  startedAt?: number;
  rulePreview?: { rules: string[]; error: string | null };
};

type Attachment = { path: string; content: string; bytes: number };
/** An image the model will see: base64 without the `data:` prefix. */
export type ImageData = import('../types').ImageAttachment;
export type QueuedComposerMessage = {
  fingerprint?: string;
  steering?: boolean;
  error?: string;
  id: string;
  text: string;
  images?: ImageData[];
};

/** Longest edge sent to the model. Bigger screenshots are scaled down —
 *  providers downscale anyway, and it keeps the payload small. */
const IMAGE_MAX_EDGE = 1568;

/** Read an image file, scaled down to IMAGE_MAX_EDGE, as base64. */
async function readImage(file: File): Promise<ImageData> {
  let image: ImageBitmap | HTMLImageElement;
  let objectUrl: string | undefined;
  try {
    image = await createImageBitmap(file);
  } catch {
    // Older desktop webviews cannot decode every format through ImageBitmap.
    objectUrl = URL.createObjectURL(file);
    const element = new Image();
    image = await new Promise<HTMLImageElement>((resolve, reject) => {
      element.onload = () => resolve(element);
      element.onerror = () => { URL.revokeObjectURL(objectUrl!); reject(new Error('This image could not be decoded.')); };
      element.src = objectUrl!;
    });
  }
  try {
    const width = image instanceof HTMLImageElement ? image.naturalWidth : image.width;
    const height = image instanceof HTMLImageElement ? image.naturalHeight : image.height;
    const scale = Math.min(1, IMAGE_MAX_EDGE / Math.max(width, height));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    const context = canvas.getContext('2d');
    if (!context) throw new Error('Image processing is unavailable.');
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    const media_type = file.type === 'image/jpeg' ? 'image/jpeg' : 'image/png';
    const url = canvas.toDataURL(media_type, 0.9);
    return { media_type, data: url.slice(url.indexOf(',') + 1), source: { name: file.name, width: canvas.width, height: canvas.height, ...(file as File & { source?: import('../types').ImageSource }).source } };
  } finally {
    if (objectUrl) URL.revokeObjectURL(objectUrl);
    if ('close' in image) image.close();
  }
}

/** Cap on how many bytes we'll inline from a single OS-picked file. Larger
 *  files still get a chip in the composer, but the inlined body is
 *  truncated with a marker so a rogue 50 MB video doesn't nuke the model's
 *  context window. Text files usually clock in well under this. */
const NATIVE_ATTACH_MAX_BYTES = 256 * 1024;


const AGENT_APPROVAL_MODES = MODES.filter((m) => ['manual', 'auto', 'edit'].includes(m.value)).map((m) => ({
  ...m,
  desc: m.value === 'edit' ? 'Approve every agent permission request.'
    : m.value === 'manual' ? 'Ask before every edit, command and web fetch.'
    : 'Auto-approve edits. Commands and web fetches ask.',
}));

export function Composer({
  disabled, busy, mode, model, providerName, cwd,
  environment, environments, envSwitching, onSwitchEnvironment,
  onSend, onQueueMessage, queuedMessages = [], onRemoveQueuedMessage, onEditQueuedMessage, onReorderQueuedMessage, onSteerQueuedMessage, onSetMode, onSetModel, onSetModelOption, onAcpModes, onAcpCurrentMode, onPickAgentMode, agentDriving, onOpenPicker, onCwdSwitched, onInterrupt, onNewChat, onOpenSettings, onRunReview, onSetGoal, onClearGoal, onCompact, goal, onRemember, onUndo,
  engine, engines, agents, agentsChecking, onCheckAgents, agentConfig, agentDescriptors, onPickProvider, onPickAgent, onConfigureAgents, sessionId, onAgentCompact, onAgentFork, onAgentReverted,
  skills, commands,
  usageRing, pendingApproval: fallbackApproval, pendingApprovals, ruleEditorCallId, onRuleEditorCancel, onActiveApprovalChange, pendingPlans, pendingQuestions, notices = [], pendingApprovalCount = 0, onAllowAllPending, pendingPlan: fallbackPlan, pendingAskUser: fallbackQuestion, onDecide, onPlanReply, onAskUserReply,
}: Props) {
  const acpModes_modes = onAcpModes ?? null;
  const acpCurrentMode = onAcpCurrentMode ?? null;
  // Use native permission modes when available. OpenCode's build/plan modes
  // describe its workflow; it and agents without permission modes use Mira's
  // three per-chat approval modes instead.
  const agentPicker = useMemo(() => {
    if (!agentDriving || !acpModes_modes || !onPickAgentMode) return null;
    const mapped = mapPosturesToModes(acpModes_modes, acpCurrentMode);
    if (engine?.driver === 'opencode' || !mapped.some((o) => o.posture.key !== 'plan')) return null;
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
  }, [agentDriving, acpModes_modes, acpCurrentMode, onPickAgentMode, engine?.driver]);
  const phone = useIsPhone();
  const [text, setText] = useState(() => cachedDraft(draftKey(sessionId)).text);
  const [attachments, setAttachmentState] = useState<Attachment[]>(() => cachedDraft(draftKey(sessionId)).attachments);
  const [images, setImageState] = useState<ImageData[]>(() => cachedDraft(draftKey(sessionId)).images);
  const [draftError, setDraftError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);
  // Send mode (Settings → General → Composer). When on, plain Enter
  // inserts a newline and only Cmd/Ctrl+Enter sends.
  const [cmdEnterSend] = useBoolPref(PREF_KEYS.composerCmdEnter, false);

  async function addImages(files: File[], owner = textOwner.current) {
    setAttachError(null);
    try {
      const read = await Promise.all(files.map(readImage));
      setImages((prev) => [...prev, ...read], owner);
    } catch (e) {
      if (textOwner.current === owner) setAttachError(`couldn't read image: ${(e as Error).message}`);
    }
  }
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attachLoading, setAttachLoading] = useState(false);
  const [filePickerOpen, setFilePickerOpen] = useState(false);
  const [slashIdx, setSlashIdx] = useState(0);
  const [slashFeedback, setSlashFeedback] = useState<string | null>(null);
  const [modelPopOpen, setModelPopOpen] = useState(false);
  // The command palette's "Switch model…" opens this same picker.
  useEffect(() => {
    const open = () => setModelPopOpen(true);
    window.addEventListener('mira:open-model-picker', open);
    return () => window.removeEventListener('mira:open-model-picker', open);
  }, []);
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
    persistText(next);
    if (mentionRef.current) {
      mentionRef.current.setText(next);
    } else {
      // Ref not attached yet (shouldn't happen after mount) — fall
      // back to React state so we don't drop the update entirely.
      setText(next);
    }
  }

  /* ---------- per-session drafts ----------
   *
   * Switching chats used to wipe whatever was half-typed: one
   * contenteditable input, one `text` state, and nothing tying it to
   * the session it belongs to.
   *
   * Two values make that safe, and the gap between them is the whole
   * bug:
   *
   *   `draftKeyFor` — the session on screen right now.
   *   `textOwner`   — the session `text` actually belongs to.
   *
   * On a switch they differ for exactly one render. So the draft is
   * written from the mutation sites (every write to the composer goes
   * through `persistText`) rather than from an effect on `text`: an
   * effect would run once more in the switching render with the *old*
   * text and the *new* key, silently replacing one chat's draft with
   * another's. `textOwner` is re-pointed by the restore before any
   * write can happen, so every write lands on the right session.
   */
  const draftKeyFor = draftKey(sessionId);
  const textOwner = useRef(draftKeyFor);

  const restoringDraft = useRef(false);
  const draftState = useRef<RichDraft>(cachedDraft(draftKeyFor));
  function applyDraft(key: string, next: RichDraft, immediate = false) {
    changeRichDraft(key, next, immediate);
    if (textOwner.current !== key) return;
    draftState.current = next;
  }
  function persistText(next: string) {
    const key = textOwner.current;
    applyDraft(key, { ...draftState.current, text: next });
  }
  function setAttachments(value: Attachment[] | ((prev: Attachment[]) => Attachment[]), key = textOwner.current) {
    const previous = key === textOwner.current ? draftState.current : cachedDraft(key);
    const next = { ...previous, attachments: typeof value === 'function' ? value(previous.attachments) : value };
    applyDraft(key, next, true);
    if (textOwner.current === key) setAttachmentState(next.attachments);
  }
  function setImages(value: ImageData[] | ((prev: ImageData[]) => ImageData[]), key = textOwner.current) {
    const previous = key === textOwner.current ? draftState.current : cachedDraft(key);
    const next = { ...previous, images: typeof value === 'function' ? value(previous.images) : value };
    applyDraft(key, next, true);
    if (textOwner.current === key) setImageState(next.images);
  }
  useLayoutEffect(() => {
    let alive = true;
    textOwner.current = draftKeyFor;
    const local = cachedDraft(draftKeyFor);
    draftState.current = local;
    setText(local.text); setAttachmentState(local.attachments); setImageState(local.images);
    restoringDraft.current = true;
    mentionRef.current?.setText(local.text);
    restoringDraft.current = false;
    setDraftError(null); setAttachError(null); setAttachLoading(false); setPreview(null);
    void restoreRichDraft(draftKeyFor).then(stored => {
      if (!alive || textOwner.current !== draftKeyFor) return;
      draftState.current = stored;
      setText(stored.text); setAttachmentState(stored.attachments); setImageState(stored.images);
      restoringDraft.current = true;
      mentionRef.current?.setText(stored.text);
      restoringDraft.current = false;
    });
    return () => { alive = false; };
  }, [draftKeyFor]);
  useEffect(() => {
    const listener = (event: Event) => {
      const detail = (event as CustomEvent<{key: string; error: string | null}>).detail;
      if (detail.key === textOwner.current) setDraftError(detail.error);
    };
    window.addEventListener('mira:draft-storage', listener);
    return () => window.removeEventListener('mira:draft-storage', listener);
  }, []);
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
    const owner = textOwner.current;
    setAttachError(null);
    setAttachLoading(true);
    try {
      const f = await readFile(path);
      // De-dupe by path — re-attaching the same file just refreshes it.
      setAttachments((prev) => [
        ...prev.filter((a) => a.path !== f.path),
        { path: f.path, content: f.content, bytes: f.bytes },
      ], owner);
    } catch (e) {
      if (textOwner.current === owner) setAttachError(String((e as Error).message));
    } finally {
      if (textOwner.current === owner) setAttachLoading(false);
    }
  }

  /** Read one browser `File` (from the OS native picker) into an
   *  Attachment. Text-shaped files inline as-is; binary/oversized files
   *  still get a chip but the inlined content is a short placeholder so
   *  the model at least sees "here's a file called foo.mp4 (14.2 MB)"
   *  even when we can't ship the bytes. */
  async function attachNativeFile(file: File, owner = textOwner.current) {
    setAttachError(null);
    setAttachLoading(true);
    try {
      if (/^image\/(png|jpeg|gif|webp)$/.test(file.type) || /\.(png|jpe?g|gif|webp)$/i.test(file.name)) {
        await addImages([file], owner);
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
      ], owner);
    } catch (e) {
      if (textOwner.current === owner) setAttachError(String((e as Error).message));
    } finally {
      if (textOwner.current === owner) setAttachLoading(false);
    }
  }

  async function attachNativeFiles(files: FileList | null) {
    const owner = textOwner.current;
    if (!files || files.length === 0) return;
    for (const f of Array.from(files)) {
      await attachNativeFile(f, owner);
    }
  }

  useEffect(() => {
    const over = (event: DragEvent) => {
      if (!event.dataTransfer?.types.includes('Files')) return;
      event.preventDefault(); event.dataTransfer.dropEffect = 'copy'; setDragging(true);
    };
    const drop = (event: DragEvent) => {
      if (!event.dataTransfer?.files.length) return;
      event.preventDefault(); setDragging(false);
      void attachNativeFiles(event.dataTransfer.files);
    };
    const leave = (event: DragEvent) => { if (!event.relatedTarget) setDragging(false); };
    window.addEventListener('dragover', over);
    window.addEventListener('drop', drop);
    window.addEventListener('dragleave', leave);
    window.addEventListener('dragend', leave);
    return () => { window.removeEventListener('dragover', over); window.removeEventListener('drop', drop); window.removeEventListener('dragleave', leave); window.removeEventListener('dragend', leave); };
  }, []);

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
    const unregister = registerAttachmentIntake(file => { void attachNativeFile(file); });
    window.addEventListener(ATTACH_FILE_EVENT, onAttach);
    return () => { unregister(); window.removeEventListener(ATTACH_FILE_EVENT, onAttach); };
  }, []);

  // A pane handing over a prompt (see `composeText`). Appended, so it
  // never clobbers a half-typed message.
  const textRef = useRef(text);
  textRef.current = text;
  useEffect(() => {
    function onCompose(e: Event) {
      const add = (e as CustomEvent<string>).detail;
      if (typeof add !== 'string' || !add) return;
      const cur = textRef.current.trimEnd();
      updateText(cur ? `${cur}\n\n${add}` : add);
      requestAnimationFrame(() => mentionRef.current?.focus());
    }
    window.addEventListener(COMPOSE_TEXT_EVENT, onCompose);
    return () => window.removeEventListener(COMPOSE_TEXT_EVENT, onCompose);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Quoted passages wait above the input as cards, not as markdown in the
  // text; they're written into the message when it's sent.
  const [quotes, setQuotes] = useState<ComposerQuote[]>([]);
  useEffect(() => {
    function onQuote(e: Event) {
      const q = (e as CustomEvent<ComposerQuote>).detail;
      if (!q?.text) return;
      setQuotes((prev) => (prev.some((p) => p.href === q.href) ? prev : [...prev, q]));
      requestAnimationFrame(() => mentionRef.current?.focus());
    }
    window.addEventListener(COMPOSE_QUOTE_EVENT, onQuote);
    return () => window.removeEventListener(COMPOSE_QUOTE_EVENT, onQuote);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function removeAttachment(path: string) {
    setAttachments((prev) => prev.filter((a) => a.path !== path));
  }

  function submit() {
    const trimmed = text.trim();
    if ((!trimmed && attachments.length === 0 && images.length === 0 && quotes.length === 0) || disabled) return;

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

    const quoted = quotes.map(quoteMarkdown).join('') + trimmed;
    const body = attachments.length > 0 ? renderAttachments(attachments, cwd) + '\n\n' + quoted : quoted;
    if (busy && onQueueMessage) onQueueMessage(body, images.length > 0 ? images : undefined);
    else if (busy) return;
    else onSend(body, images.length > 0 ? images : undefined);
    updateText('');
    clearRichDraft(textOwner.current);
    draftState.current = { text: '', attachments: [], images: [] };
    setAttachmentState([]);
    setImageState([]);
    setQuotes([]);
    setAttachError(null);
    setSlashFeedback(null);
  }

  const modeLabel = MODES.find((m) => m.value === mode)?.label ?? mode;

  // Keep every pending request addressable. Selecting a notice does not
  // dismiss it, approve it, or discard the ordinary composer draft.
  const approvals = pendingApprovals ?? (fallbackApproval ? [fallbackApproval] : []);
  const plans = pendingPlans ?? (fallbackPlan ? [fallbackPlan] : []);
  const questions = pendingQuestions ?? (fallbackQuestion ? [fallbackQuestion] : []);
  const noticeItems: ComposerNotice[] = [
    ...questions.map(item => ({ id: `question:${item.callId}`, kind: 'question' as const, title: 'Answer a question', detail: item.proposal.questions[0]?.question })),
    ...plans.map(item => ({ id: `plan:${item.callId}`, kind: 'plan' as const, title: 'Review the proposed plan' })),
    ...approvals.map(item => ({ id: `approval:${item.callId}`, kind: 'approval' as const, title: 'Permission requested', detail: item.call.function.name })),
    ...notices,
  ];
  const [selectedNotice, setSelectedNotice] = useState<string | null>(null);
  useEffect(() => { if (ruleEditorCallId) setSelectedNotice(`approval:${ruleEditorCallId}`); }, [ruleEditorCallId]);
  const selected = noticeItems.find(item => item.id === selectedNotice) ?? noticeItems[0];
  const pendingAskUser = questions.find(item => selected?.id === `question:${item.callId}`);
  const pendingPlan = plans.find(item => selected?.id === `plan:${item.callId}`);
  const pendingApproval = approvals.find(item => selected?.id === `approval:${item.callId}`);
  const activePromptKind = pendingAskUser ? 'ask_user' : pendingPlan ? 'plan' : pendingApproval ? 'approval' : null;
  useEffect(() => { onActiveApprovalChange?.(pendingApproval?.callId ?? null); return () => onActiveApprovalChange?.(null); }, [pendingApproval?.callId, onActiveApprovalChange]);

  return (
    <div className="flex flex-col items-center gap-1.5 px-4 pb-4 pt-2 max-md:px-2 max-md:pb-2">
      {draftError && <p role="alert" className="w-full max-w-3xl px-2 text-[12px] text-amber-700 dark:text-amber-400">{draftError}</p>}
      <QueuedMessageStack
        key={sessionId ?? 'unsaved'}
        items={queuedMessages}
        onEdit={onEditQueuedMessage}
        onReorder={onReorderQueuedMessage}
        canSteer={!disabled && !!busy && (engine?.capabilities?.steering === 'native' || engine?.capabilities?.steering === 'safe_boundary')}
        onRemove={onRemoveQueuedMessage}
        onSteer={onSteerQueuedMessage}

      />
      <ComposerContextDock
        environment={environment ?? null}
        environments={environments ?? []}
        envSwitching={envSwitching ?? null}
        envDisabled={busy || disabled}
        onSwitchEnvironment={onSwitchEnvironment}
        usageRing={usageRing}
        cwd={cwd}
        onCwdSwitched={onCwdSwitched}
      />
      <form
        className={cn(
          'relative w-full max-w-3xl flex flex-col gap-1.5 rounded-[22px] bg-white shadow-[0_12px_30px_-26px_rgba(15,23,42,0.55)] ring-1 ring-border/55 backdrop-blur-xl p-2.5 transition-colors dark:bg-secondary/85 dark:shadow-[0_14px_36px_-28px_rgba(0,0,0,0.8)]',
          dragging && 'bg-mira-blue/[0.06] ring-1 ring-mira-blue/45',
        )}
        onSubmit={(e) => { e.preventDefault(); submit(); }}
        onDragOverCapture={(e) => {
          if (!e.dataTransfer.types.includes('Files')) return;
          e.preventDefault();
          e.stopPropagation();
          setDragging(true);
        }}
        onDragLeaveCapture={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDropCapture={(e) => {
          setDragging(false);
          if (e.dataTransfer.files.length === 0) return;
          e.preventDefault();
          e.stopPropagation();
          void attachNativeFiles(e.dataTransfer.files);
        }}
      >
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-[22px] text-[13px] font-medium text-mira-blue">
            Drop files or images to attach
          </div>
        )}
        {selected && <ComposerNoticeStack items={noticeItems} selected={selected.id} onSelect={setSelectedNotice} />}
        {notices.map(notice => <div key={notice.id} hidden={selected?.id !== notice.id}>{notice.content}</div>)}
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
                /><ImageAttachmentDetails image={img} />
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

        {!activePromptKind && quotes.length > 0 && (
          <div className="flex flex-col gap-1.5 px-1.5 pt-1">
            {quotes.map((q) => (
              <QuoteCard key={q.id} quote={q} onRemove={() => setQuotes((prev) => prev.filter((p) => p.id !== q.id))} />
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

        {/* Keep unsent answers and edited plans mounted when switching notices. */}
        {plans.map(plan => <m.div key={`plan:${plan.callId}`} hidden={pendingPlan?.callId !== plan.callId} initial={false} animate={{opacity:pendingPlan?.callId === plan.callId ? 1 : 0}} transition={{duration:.15}}>
          {onPlanReply && <EmbeddedPlanCard proposal={plan.proposal} onApprove={steps => onPlanReply(plan.callId, true, steps)} onCancel={note => onPlanReply(plan.callId, false, undefined, note || undefined)} />}
        </m.div>)}
        {questions.map(question => <m.div key={`question:${question.callId}`} hidden={pendingAskUser?.callId !== question.callId} initial={false} animate={{opacity:pendingAskUser?.callId === question.callId ? 1 : 0}} transition={{duration:.15}}>
          {onAskUserReply && <EmbeddedAskUserCard asker={engine?.kind === 'agent' ? engine.display_name : 'Mira'} proposal={question.proposal} onSubmit={answers => onAskUserReply(question.callId, {cancelled:false,answers})} onCancel={() => onAskUserReply(question.callId, {cancelled:true})} />}
        </m.div>)}

        {!activePromptKind && (
          <div className="relative">
            <MentionInput
              handleRef={mentionRef}
              onPasteImages={(files) => void addImages(files)}
              value={text}
              onChange={(next) => { setText(next); if (!restoringDraft.current) persistText(next); setSlashFeedback(null); }}
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
                      : phone
                      ? 'Ask mira anything'
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

        {/* Bottom slot: the approval card when one is pending (where the
            model picker used to sit), the toolbar otherwise. */}
        {approvals.length > 1 && onDecide && <ApprovalQueue approvals={approvals} activeId={pendingApproval?.callId} onFocus={id => setSelectedNotice(`approval:${id}`)} onDecide={(id, allow) => onDecide(id, allow, 'once')} />}
        {approvals.map((approval, index) => <div key={`approval:${approval.callId}`} hidden={pendingApproval?.callId !== approval.callId}>
          {onDecide && <EmbeddedApprovalCard position={index + 1} openRuleEditor={ruleEditorCallId === approval.callId} onRuleEditorCancel={onRuleEditorCancel} approval={approval} queued={pendingApprovalCount} onAllowAll={onAllowAllPending} onDecide={(allow, scope, rules) => onDecide(approval.callId, allow, scope, rules)} />}
        </div>)}
        {(!activePromptKind && (
          <div className="flex items-center gap-1.5 px-1">
            <AttachMenu onAttachFile={openNativeFiles} loading={attachLoading} onScreenshot={async () => {
              const owner = textOwner.current;
              setAttachLoading(true); setAttachError(null);
              try { const file = await captureScreenshot(); if (file) await attachNativeFile(file, owner); }
              catch (error) { if (textOwner.current === owner) setAttachError(error instanceof Error ? error.message : String(error)); }
              finally { if (textOwner.current === owner) setAttachLoading(false); }
            }} />

            {/* At phone width the row keeps only what's used from a phone:
                typing "/" still opens commands, and projects are in the
                sidebar drawer. */}
            <span className="contents max-md:hidden">
              <SlashButton onClick={() => updateText(text.startsWith('/') || text.startsWith('@') ? text : '/' + text)} />
            </span>

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

            <span className="contents max-md:hidden">
              <ProjectChip cwd={cwd} onClick={onOpenPicker} />
            </span>

            <span className="flex-1 max-md:min-w-1" />

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
            ) : (
              <ModePicker mode={mode} label={modeLabel} onPick={onSetMode}
                modes={agentDriving ? AGENT_APPROVAL_MODES : undefined} />
            )}

            {busy && !text.trim() && attachments.length === 0 && images.length === 0 && quotes.length === 0 ? (
              <button
                type="button"
                onClick={onInterrupt}
                className="flex size-8 shrink-0 items-center justify-center rounded-full bg-mira-error text-mira-on-accent transition-colors hover:brightness-110 touch:size-10"
                title="Stop"
                aria-label="Stop"
              >
                <Square className="size-3.5" style={{ fill: 'currentColor' }} />
              </button>
            ) : (
              <button
                type="submit"
                disabled={disabled || (!text.trim() && attachments.length === 0 && images.length === 0 && quotes.length === 0)}
                className="flex size-8 shrink-0 items-center justify-center rounded-full bg-foreground text-background transition-opacity hover:opacity-90 disabled:opacity-35 touch:size-10"
                title={busy ? 'Queue message' : (cmdEnterSend ? 'Send (⌘/Ctrl+Enter)' : 'Send (Enter)')}
                aria-label={busy ? 'Queue message' : 'Send'}
              >
                <ArrowUp className="size-4" />
              </button>
            )}
          </div>
        )
        )}
      </form>

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
        data-composer-attachments=""
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
        'rounded-2xl border border-fg/[0.07] bg-popover/95 dark:bg-[#1f2024]/95 backdrop-blur-md',
        'shadow-[0_20px_50px_-16px_rgba(0,0,0,0.85)] ring-1 ring-shade/40',
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
                  gi > 0 && 'mt-1 border-t border-fg/[0.05]',
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
                    active ? 'bg-fg/[0.06]' : 'hover:bg-fg/[0.035]',
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
                    <span className="shrink-0 self-center rounded border border-fg/[0.08] px-1.5 text-[10px] uppercase tracking-wide text-muted-foreground/70">
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
    <span className="flex size-3.5 shrink-0 items-center justify-center rounded-[3px] bg-fg/10 text-[9px] font-semibold text-foreground/80">
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
  onAttachFile, loading, onScreenshot,
}: { onAttachFile: () => void; loading: boolean; onScreenshot: () => Promise<void> }) {
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
            hint="Capture a screen region or window"
            onClick={() => { setOpen(false); void onScreenshot(); }}
            disabled={loading}
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
            'inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1.5 text-[12.5px] leading-none transition-colors touch:p-2.5',
            mode === 'edit' || mode === 'yolo' ? 'text-orange-600 dark:text-orange-400 hover:text-orange-600 dark:hover:text-orange-400' : 'text-muted-foreground',
            disabled ? 'cursor-default opacity-80' : 'hover:bg-mira-elev2',
            !disabled && mode !== 'edit' && mode !== 'yolo' && 'hover:text-foreground',
          )}
        >
          <ApprovalModeIcon mode={mode} />
          {/* Icon only on a phone; the menu names every mode. */}
          <span className="block leading-none max-md:sr-only">{label}</span>
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
              <span className={cn("flex items-center gap-2 text-[13px] leading-none", (m.value === 'edit' || m.value === 'yolo') && "text-orange-600 dark:text-orange-400")}><ApprovalModeIcon mode={m.value} /><span>{m.label}</span>{m.value === mode && <PhCheck aria-hidden="true" className="ml-auto size-3.5 shrink-0" />}</span>
              <span className={cn("ml-[22px] mt-1 text-[11.5px]", m.value === 'edit' || m.value === 'yolo' ? "text-orange-600 dark:text-orange-400" : "text-muted-foreground")}>{m.desc}</span>
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

function QueuedMessageStack({
  items,
  onRemove,
  onSteer,
  onEdit,
  onReorder,
  canSteer = true,
}: {
  items: QueuedComposerMessage[];
  canSteer?: boolean;
  onRemove?: (id: string) => void;
  onSteer?: (id: string) => void;
  onEdit?: (item: QueuedComposerMessage, text: string) => Promise<void>;
  onReorder?: (id: string, beforeId: string | null) => Promise<void>;
}) {
  const [expanded, setExpanded] = useState(false);
  const [editing, setEditing] = useState<QueuedComposerMessage | null>(null);
  const [editText, setEditText] = useState('');
  const [saving, setSaving] = useState(false);
  const [moving, setMoving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editError, setEditError] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const dragId = useRef<string | null>(null);
  const armedId = useRef<string | null>(null);
  const listId = useId();
  const reduceMotion = useReducedMotion();
  const editable = (item: QueuedComposerMessage) => !item.steering && !item.error && !!item.fingerprint;
  async function move(id: string, beforeId: string | null) {
    if (!onReorder || moving || saving) return;
    setMoving(true); setError(null);
    try { await onReorder(id, beforeId); }
    catch (error) { setError(error instanceof Error ? error.message : 'Could not reorder the queue.'); }
    finally { setMoving(false); }
  }
  async function save() {
    if (!editing || !onEdit || saving) return;
    const parsed = parseSentAttachments(editing.text);
    const prefix = parsed.attachments.length ? editing.text.slice(0, editing.text.length - parsed.text.length) : '';
    setSaving(true); setEditError(null);
    try { await onEdit(editing, prefix + editText); setEditing(null); }
    catch (error) { setEditError(error instanceof Error ? error.message : 'Could not save this edit.'); }
    finally { setSaving(false); }
  }
  if (items.length === 0 && !editing) return null;
  const shown = expanded ? items : items.slice(0, 4);
  const extra = items.length - shown.length;
  return (
    <>
    {items.length > 0 && <m.div
      layout
      className="-mb-3 w-full max-w-3xl px-7 sm:px-8"
      initial={{ opacity: 0, y: reduceMotion ? 0 : 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 8 }}
      transition={{ duration: reduceMotion ? 0 : 0.18, ease: [0.4, 0, 0.2, 1] }}
    >
      <div className="overflow-hidden rounded-t-[18px] rounded-b-none border border-b-0 border-border/55 bg-white pb-4 pt-1.5 shadow-[0_8px_24px_-22px_rgba(15,23,42,0.45)] dark:border-fg/[0.055] dark:bg-secondary/85 dark:shadow-[0_10px_28px_-24px_rgba(0,0,0,0.75)] backdrop-blur-xl">
        <div id={listId} role="list" aria-label="Queued messages" className="max-h-64 overflow-y-auto">
        <AnimatePresence initial={false}>
          {shown.map((item, index) => (
            <m.div
              layout={!reduceMotion}
              key={item.id}
              initial={{ opacity: 0, height: 0, y: 6 }}
              animate={{ opacity: 1, height: 'auto', y: 0 }}
              exit={{ opacity: 0, height: 0, y: -4 }}
              transition={{ duration: reduceMotion ? 0 : 0.18, ease: [0.4, 0, 0.2, 1] }}
              role="listitem"
              draggable={!!onReorder && editable(item) && !moving && !saving}
              onDragStartCapture={event => {
                if (armedId.current !== item.id) { event.preventDefault(); return; }
                dragId.current = item.id;
                event.dataTransfer.setData('application/x-mira-queued-message', item.id);
                event.dataTransfer.effectAllowed = 'move';
              }}
              onDragOver={event => {
                if (!dragId.current || dragId.current === item.id || !editable(item)) return;
                event.preventDefault(); event.dataTransfer.dropEffect = 'move'; setDropTarget(item.id);
              }}
              onDrop={event => {
                const id = dragId.current;
                if (!id || id === item.id || !editable(item)) return;
                event.preventDefault(); dragId.current = null; armedId.current = null; setDropTarget(null);
                void move(id, item.id);
              }}
              onDragEndCapture={() => { dragId.current = null; armedId.current = null; setDropTarget(null); }}
              className={cn('flex min-w-0 items-center gap-2 px-3.5 py-1.5 text-[13px]', dropTarget === item.id && 'ring-1 ring-inset ring-mira-blue')}
            >
              {onReorder && editable(item) ? <button type="button" aria-label="Drag to reorder queued message" title="Drag to reorder; use the options menu to move with the keyboard" disabled={moving || saving} onPointerDown={() => { armedId.current = item.id; }} onPointerUp={() => { armedId.current = null; }} className="flex size-5 shrink-0 cursor-grab items-center justify-center text-muted-foreground/70 active:cursor-grabbing"><GripVertical className="size-3.5" /></button> : <CornerDownRight className="size-3.5 shrink-0 text-muted-foreground/55" />}
              <QueuedThumb item={item} />
              <span className="min-w-0 flex-1 truncate text-foreground/90">
                {queueTitle(item)}{item.error && <span role="status" className="ml-2 text-xs text-muted-foreground" title={item.error}>{item.error}</span>}
              </span>
              <button
                type="button"
                onPointerDown={(e) => e.preventDefault()}
                disabled={item.steering || !!item.error || !canSteer || saving || moving}
                onClick={() => onSteer?.(item.id)}
                className="inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-1 text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
                title={canSteer ? "Send as input to the active turn" : "Steering is unavailable; this message will run after the current turn"}
              >
                <CornerDownRight className="size-3.5" />
                <span>{item.steering ? 'Sending…' : 'Steer'}</span>
              </button>
              <button
                type="button"
                onPointerDown={(e) => e.preventDefault()}
                onClick={() => onRemove?.(item.id)}
                disabled={item.steering || saving || moving}
                aria-label="Remove queued message"
                className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground/80 transition-colors hover:bg-fg/[0.06] hover:text-foreground"
              >
                <Trash className="size-3.5" />
              </button>
              {!item.steering && <QueuedMessageMenu
                item={item}
                onEdit={editable(item) && onEdit && !saving && !moving ? item => { setEditing(item); setEditText(parseSentAttachments(item.text).text); setEditError(null); } : undefined}
                onMoveUp={index > 0 && editable(item) && editable(items[index - 1]) && onReorder && !moving && !saving ? () => void move(item.id, items[index - 1].id) : undefined}
                onMoveDown={index < items.length - 1 && editable(item) && editable(items[index + 1]) && onReorder && !moving && !saving ? () => void move(item.id, items[index + 2]?.id ?? null) : undefined}
                onRemove={saving || moving ? undefined : onRemove}
              />}
            </m.div>
          ))}
        </AnimatePresence>
        </div>
        {items.length > 4 && <button type="button" aria-expanded={expanded} aria-controls={listId} onClick={() => setExpanded(value => !value)} className="flex w-full items-center gap-1.5 px-10 py-1 text-left text-[12px] text-muted-foreground hover:text-foreground">{expanded ? 'Show fewer messages' : `Show ${extra} more queued`}<ChevronDown className={cn('size-3 transition-transform', expanded && 'rotate-180')} /></button>}
        {error && <p role="alert" className="px-4 py-1 text-xs text-destructive">{error}</p>}
      </div>
    </m.div>}
    <Dialog open={!!editing} onOpenChange={open => { if (!open && !saving) setEditing(null); }}>
      <DialogContent onEscapeKeyDown={event => { if (saving) event.preventDefault(); }} onPointerDownOutside={event => { if (saving) event.preventDefault(); }}>
        <DialogTitle>Edit queued message</DialogTitle>
        <DialogDescription>The original stays in the queue until your changes are saved.</DialogDescription>
        <textarea aria-label="Queued message text" value={editText} onChange={event => setEditText(event.target.value)} disabled={saving} className="min-h-32 w-full resize-y rounded-lg border border-border bg-background p-3 text-[13px] outline-none focus:ring-1 focus:ring-mira-blue" />
        {editing && <div className="flex items-center gap-2 text-xs text-muted-foreground"><QueuedThumb item={editing} />{editing.images?.length ? `${editing.images.length} image attachment${editing.images.length === 1 ? '' : 's'} retained` : parseSentAttachments(editing.text).attachments.map(file => file.filename).join(', ')}</div>}
        {editError && <p role="alert" className="text-xs text-destructive">{editError}</p>}
        <div className="flex justify-end gap-2"><button type="button" disabled={saving} onClick={() => setEditing(null)} className="rounded-md px-3 py-1.5 text-[13px] hover:bg-secondary">Cancel</button><button type="button" disabled={saving || (!editText.trim() && !editing?.images?.length && !parseSentAttachments(editing?.text ?? '').attachments.length)} onClick={() => void save()} className="rounded-md bg-primary px-3 py-1.5 text-[13px] text-primary-foreground disabled:opacity-50">{saving ? 'Saving…' : 'Save changes'}</button></div>
      </DialogContent>
    </Dialog>
    </>
  );
}

function queueTitle(item: QueuedComposerMessage) {
  const { text, attachments } = parseSentAttachments(item.text);
  const prose = text.replace(/\s+/g, ' ').trim();
  if (prose) return prose;
  const n = item.images?.length ?? 0;
  if (n) return n === 1 ? 'Image attachment' : `${n} image attachments`;
  if (attachments.length) return attachments.length === 1 ? attachments[0].filename : `${attachments.length} file attachments`;
  return 'Queued message';
}

function QueuedThumb({ item }: { item: QueuedComposerMessage }) {
  const first = item.images?.[0];
  if (first) {
    return (
      <img
        src={`data:${first.media_type};base64,${first.data}`}
        alt=""
        className="size-7 shrink-0 rounded-md object-cover ring-1 ring-fg/[0.08]"
        draggable={false}
      />
    );
  }
  const attachments = parseSentAttachments(item.text).attachments;
  if (!attachments.length) return null;
  return <span title={attachments.map(file => file.filename).join(', ')} className="flex size-7 shrink-0 items-center justify-center rounded-md bg-fg/[0.06] text-muted-foreground"><FileIcon className="size-3.5" /></span>;
}

function QueuedMessageMenu({
  item,
  onEdit,
  onRemove,
  onMoveUp,
  onMoveDown,
}: {
  item: QueuedComposerMessage;
  onEdit?: (item: QueuedComposerMessage) => void;
  onRemove?: (id: string) => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Queued message options"
          className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground/70 transition-colors hover:bg-fg/[0.06] hover:text-foreground"
          title="Queued message options"
        >
          <MoreHorizontal className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-44 p-1.5" align="end">
        {onMoveUp && <MenuButton icon={<PhArrowUp className="size-3.5" />} label="Move up" onClick={() => { setOpen(false); onMoveUp(); }} />}
        {onMoveDown && <MenuButton icon={<PhArrowDown className="size-3.5" />} label="Move down" onClick={() => { setOpen(false); onMoveDown(); }} />}
        {onEdit && <MenuButton
          icon={<PenLine className="size-3.5" />}
          label="Edit"
          hint="Keep its place in the queue"
          onClick={() => {
            setOpen(false);
            onEdit?.(item);
          }}
        />}
        <MenuButton
          icon={<Trash className="size-3.5" />}
          label="Remove"
          hint="Cancel this queued message"
          onClick={() => {
            setOpen(false);
            onRemove?.(item.id);
          }}
        />
      </PopoverContent>
    </Popover>
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


function ComposerContextDock({
  environment,
  environments,
  envSwitching,
  envDisabled,
  onSwitchEnvironment,
  usageRing,
  cwd,
  onCwdSwitched,
}: {
  environment: EnvironmentStatus | null;
  environments: EnvironmentInfo[];
  envSwitching: string | null;
  envDisabled: boolean;
  onSwitchEnvironment?: (target: string) => void;
  usageRing?: UsageRingData | null;
  cwd: string;
  onCwdSwitched?: (path: string, sessionId?: string) => void;
}) {
  return (
    <m.div
      layout
      // Environment, usage and worktree: desk-side controls, and on a phone
      // the height is better spent on the transcript.
      className="-mb-3 w-full max-w-3xl px-4 sm:px-5 max-md:hidden"
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
    >
      <div className="flex min-h-10 items-center gap-2 rounded-t-[18px] rounded-b-none border border-b-0 border-border/55 bg-white px-3 pb-4 pt-1.5 shadow-[0_8px_24px_-22px_rgba(15,23,42,0.45)] dark:border-fg/[0.055] dark:bg-secondary/85 dark:shadow-[0_10px_28px_-24px_rgba(0,0,0,0.75)] backdrop-blur-xl">
        <EnvironmentChip
          status={environment}
          environments={environments}
          switching={envSwitching}
          disabled={envDisabled}
          onSwitch={onSwitchEnvironment}
        />
        <span className="min-w-4 flex-1" />
        {usageRing && <UsageRing data={usageRing} />}
        <WorktreeChip cwd={cwd} onCwdSwitched={onCwdSwitched} />
      </div>
    </m.div>
  );
}

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
          <span className="truncate">{switching ? 'switching…' : (status.current === 'e2b' ? 'Cloud Sandbox' : status.current)}</span>
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
                    <span className="truncate">{e.name === 'e2b' ? 'Cloud Sandbox' : e.name}</span>
                    {e.backend !== 'local' && e.backend !== e.name && (
                      <span className="rounded-sm bg-secondary px-1 text-[9.5px] uppercase tracking-wider text-muted-foreground">{e.backend === 'e2b' ? 'Cloud Sandbox' : e.backend}</span>
                    )}
                    {parked && (
                      <span className="rounded-sm bg-mira-blue/15 px-1 text-[9.5px] uppercase tracking-wider text-mira-blue" title="paused — resumes quickly">paused</span>
                    )}
                  </span>
                  {e.description && (
                    <span className="block truncate text-[11px] text-muted-foreground/70">{e.description.replace(/\bE2B\b/g, 'Cloud Sandbox')}</span>
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

  const setupSequence = useRef(0);
  async function prepareWorkspace(branch: string, existingPath?: string) {
    if (!branch.trim() || busy) return;
    const sequence = ++setupSequence.current;
    const setup: WorkspaceSetup = { id: `workspace-${Date.now()}-${sequence}`, branch, phase: 'creating', startedAt: Date.now() };
    let cancelled = false;
    let createdPath = existingPath;
    let stage: 'creating' | 'opening' = existingPath ? 'opening' : 'creating';
    const report = (phase: WorkspaceSetup['phase'], error?: string) => reportWorkspaceSetup({ ...setup, phase, error, failedStage: phase === 'failed' ? stage : undefined,
      cancel: phase === 'creating' ? () => { cancelled = true; report('cancelled'); } : undefined,
      workLocally: phase === 'creating' ? () => { cancelled = true; report('cancelled'); } : phase === 'failed' ? () => { void switchTo(cwd); report('cancelled'); } : undefined,
      retry: phase === 'failed' ? () => void prepareWorkspace(branch, createdPath) : undefined,
    });
    setBusy(true); setLoadError(null); setOpen(false); report(stage);
    try {
      createdPath ??= (await createWorktree(branch)).path;
      if (cancelled || sequence !== setupSequence.current) return;
      stage = 'opening'; report('opening');
      const { session_id } = await putCwd(createdPath);
      onCwdSwitched?.(createdPath, session_id); setNewBranch(''); report('done');
      await refresh();
    } catch (error) {
      if (!cancelled && sequence === setupSequence.current) { const message = error instanceof Error ? error.message : String(error); setLoadError(message); report('failed', message); }
    } finally { if (sequence === setupSequence.current) setBusy(false); }
  }
  async function createAndSwitch() { await prepareWorkspace(newBranch.trim()); }
  async function createAndSwitchTo(branch: string) { await prepareWorkspace(branch); }

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
          className="mx-1.5 mt-1.5 inline-flex items-center gap-1.5 rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-3 py-1.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
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
            {dirty ? 'Approve will run the edited plan' : <span className="touch:hidden">⌘↵ to approve</span>}
          </span>
          <button
            type="button"
            onClick={() => onCancel(note)}
            className="rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-4 py-1.5 text-[11.5px] font-medium touch:px-5 touch:py-2.5 touch:text-[13.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onApprove(steps)}
            disabled={!canApprove}
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold touch:px-5 touch:py-2.5 touch:text-[13.5px] transition-all',
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
      <m.div
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
                  active ? 'bg-foreground text-background' : 'bg-secondary/40 hover:bg-secondary/60',
                )}
              >
                <span
                  className={cn(
                    'mt-[3px] flex size-[14px] shrink-0 items-center justify-center transition-colors',
                    multi ? 'rounded-[5px]' : 'rounded-full',
                    active ? 'bg-background' : 'bg-background/60 ring-1 ring-inset ring-border',
                  )}
                >
                  {active && multi && <PhCheck className="size-2.5 text-foreground" strokeWidth={2.5} />}
                  {active && !multi && <span className="size-1.5 rounded-full bg-foreground" />}
                </span>
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <div className="flex items-center gap-1.5">
                    <span className={cn('text-[13px] font-medium', active ? 'text-background' : 'text-foreground')}>
                      {opt.label}
                    </span>
                    {opt.recommended && (
                      <span className={cn('text-[10px] font-medium uppercase tracking-wider', active ? 'text-background/65' : 'text-muted-foreground/80')}>
                        · Recommended
                      </span>
                    )}
                  </div>
                  {opt.description && (
                    <span className={cn('text-[11.5px] leading-snug', active ? 'text-background/70' : 'text-muted-foreground')}>
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
      </m.div>
      </AnimatePresence>

      <div className="border-t border-border/30" />

      <div className="flex items-center gap-1.5 px-1.5 py-2.5">
        {clampedIdx > 0 ? (
          <button
            type="button"
            onClick={() => { setDir(-1); setIdx((i) => Math.max(0, i - 1)); }}
            className="inline-flex items-center gap-1 rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-4 py-1.5 text-[11.5px] font-medium touch:px-5 touch:py-2.5 touch:text-[13.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
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
          className="ml-auto rounded-full bg-fg/[0.04] ring-1 ring-fg/[0.08] px-4 py-1.5 text-[11.5px] font-medium touch:px-5 touch:py-2.5 touch:text-[13.5px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
        >
          Skip
        </button>
        <button
          type="button"
          onClick={advance}
          disabled={!currentReady}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-[11.5px] font-semibold touch:px-5 touch:py-2.5 touch:text-[13.5px] transition-all',
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


function ApprovalDetails({ tool, args, fallback }: { tool: string; args: Record<string, unknown> | null; fallback: string }) {
  const command = typeof args?.command === 'string' ? args.command : null;
  if (command) {
    return (
      <pre className="m-0 max-h-[22vh] overflow-auto rounded-md border border-border/35 bg-mira-elev1/45 px-3 py-2 font-mono text-[12px] leading-relaxed">
        {command.replace(/\\n/g, '\n').split('\n').map((line, i) => (
          <div key={i} className="flex gap-2 whitespace-pre-wrap break-all">
            <span className="shrink-0 select-none text-muted-foreground/60">{i === 0 ? '$' : ' '}</span>
            <span className="text-foreground/90">{line || ' '}</span>
          </div>
        ))}
      </pre>
    );
  }
  const rows = approvalRows(tool, args);
  if (rows.length > 0) {
    return (
      <div className="rounded-md border border-border/35 bg-mira-elev1/45 px-3 py-2 text-[12.5px]">
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5">
          {rows.map((row) => (
            <div key={row.label} className="contents">
              <dt className="select-none text-muted-foreground/65">{row.label}</dt>
              <dd className="min-w-0 break-words font-mono text-foreground/85">{row.value}</dd>
            </div>
          ))}
        </dl>
      </div>
    );
  }
  return (
    <pre className="m-0 max-h-[22vh] overflow-auto whitespace-pre-wrap rounded-md bg-background/55 px-3 py-2 font-mono text-xs text-muted-foreground">
      {fallback}
    </pre>
  );
}

function approvalRows(tool: string, args: Record<string, unknown> | null): { label: string; value: string }[] {
  if (!args) return [];
  const rows: { label: string; value: string }[] = [];
  const add = (label: string, value: unknown) => {
    if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') return;
    const text = String(value).trim();
    if (text) rows.push({ label, value: text.length > 160 ? `${text.slice(0, 160)}…` : text });
  };
  const lower = tool.toLowerCase();
  if (lower.includes('edit') || lower.includes('write') || lower.includes('file')) {
    add('file', args.path ?? args.file_path ?? args.filePath ?? args.file);
    return rows;
  }
  if (lower.includes('fetch')) add('url', args.url);
  if (lower.includes('search') || lower.includes('grep')) add('query', args.query ?? args.pattern);
  add('target', args.path ?? args.file_path ?? args.url ?? args.query ?? args.pattern);
  return rows;
}

function safeJson(text: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(text || '{}');
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function ApprovalQueue({ approvals, activeId, onFocus, onDecide }: { approvals: PendingApproval[]; activeId?: string; onFocus: (id: string) => void; onDecide: (id: string, allow: boolean) => void }) {
  const [excluded, setExcluded] = useState<Set<string>>(new Set());
  const [now, setNow] = useState(Date.now());
  const firstSeen = useRef<Record<string, number>>({});
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, []);
  const groups = new Map<string, PendingApproval[]>();
  for (const approval of approvals) {
    firstSeen.current[approval.callId] ??= Date.now();
    const agent = agentRequestOf(approval.call);
    const args = agent?.input ?? safeJson(approval.call.function.arguments) ?? {};
    const tool = agent?.tool ?? approval.call.function.name;
    const path = typeof args.path === 'string' ? args.path : typeof args.file_path === 'string' ? args.file_path : '';
    const folder = path.includes('/') ? path.slice(0, path.lastIndexOf('/') + 1) : '';
    const key = `${tool}${folder ? ` in ${folder}` : ''}`;
    groups.set(key, [...(groups.get(key) ?? []), approval]);
  }
  const selected = approvals.filter(approval => !excluded.has(approval.callId));
  return <section aria-label="Approval queue" className="mb-2 rounded-xl border border-border/70 bg-background/40 p-2">
    <div className="mb-2 flex items-center justify-between gap-2"><span className="text-xs font-medium">{approvals.length} pending approvals</span><button type="button" onClick={() => setExcluded(selected.length === approvals.length ? new Set(approvals.map(item => item.callId)) : new Set())} className="text-[11px] text-muted-foreground">{selected.length === approvals.length ? 'Deselect all' : 'Select all'}</button></div>
    <div className="max-h-48 overflow-auto space-y-2">{Array.from(groups, ([label, items]) => <div key={label}>
      <p className="px-1 text-[11px] text-muted-foreground">{items.length} × {label}</p>
      {items.map(item => {
        const seconds = Math.max(0, Math.floor((now - (item.startedAt ?? firstSeen.current[item.callId])) / 1000));
        const agent = agentRequestOf(item.call);
        const args = agent?.input ?? safeJson(item.call.function.arguments) ?? {};
        const summary = String(args.command ?? args.path ?? args.file_path ?? args.query ?? item.call.function.name);
        return <div key={item.callId} className={cn('flex items-center gap-2 rounded-lg px-2 py-1.5', activeId === item.callId && 'bg-mira-blue/10')}>
          <input type="checkbox" data-approval-queue="true" onFocus={() => onFocus(item.callId)} aria-label={`Select ${summary}`} checked={!excluded.has(item.callId)} onChange={event => setExcluded(previous => { const next = new Set(previous); if (event.target.checked) next.delete(item.callId); else next.add(item.callId); return next; })} />
          <button type="button" aria-pressed={activeId === item.callId} onClick={() => onFocus(item.callId)} onFocus={() => onFocus(item.callId)} className="min-w-0 flex-1 truncate text-left font-mono text-[11.5px]" title={summary}>{summary}</button>
          <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">{seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`}</span>
        </div>;
      })}
    </div>)}</div>
    <div className="mt-2 flex justify-end gap-2"><button type="button" disabled={!selected.length} onClick={() => selected.forEach(item => onDecide(item.callId, false))} className="rounded-full border border-border px-4 py-2 text-xs font-medium disabled:opacity-40">Deny {selected.length === approvals.length ? 'all' : 'selected'} ({selected.length})</button><button type="button" disabled={!selected.length} onClick={() => selected.forEach(item => onDecide(item.callId, true))} className="rounded-full bg-mira-blue px-4 py-2 text-xs font-medium text-mira-on-accent disabled:opacity-40">Approve {selected.length === approvals.length ? 'all' : 'selected'} ({selected.length})</button></div>
  </section>;
}

/** Tool approval rendered directly inside the Composer. */
function EmbeddedApprovalCard({
  approval,
  openRuleEditor,
  onRuleEditorCancel,
  onDecide,
  queued = 1,
  position = 1,
}: {
  approval: PendingApproval;
  openRuleEditor?: boolean;
  onRuleEditorCancel?: () => void;
  onDecide: (allow: boolean, scope?: ApprovalScope, rules?: string[]) => void;
  /** Requests waiting, this one included. */
  queued?: number;
  position?: number;
  onAllowAll?: () => void;
}) {
  const { call, preview } = approval;
  const [editingRule, setEditingRule] = useState(false);
  useEffect(() => { if (openRuleEditor) setEditingRule(true); }, [openRuleEditor]);
  const [ruleDraft, setRuleDraft] = useState<string | null>(null);
  const rules = (ruleDraft ?? approval.rulePreview?.rules.join('\n') ?? '').split('\n').map(rule => rule.trim()).filter(Boolean);
  // The keys App listens for, as the user has them bound.
  const keybindings = useKeybindings();
  const allowKey = shortcutLabelForCommand(keybindings, 'approval.accept', { context: { approvalOpen: true } }) ?? 'Y';
  const denyKey = shortcutLabelForCommand(keybindings, 'approval.reject', { context: { approvalOpen: true } }) ?? 'N';
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
          {queued > 1 ? `${position} of ${queued} awaiting` : 'awaiting approval'}
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
          <ApprovalDetails tool={agent ? agent.tool : call.function.name} args={agent ? agent.input : safeJson(call.function.arguments)} fallback={prettyArgs} />
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
            { id: 'deny', label: 'Deny', title: 'Deny', kbd: denyKey },
            { id: 'always', label: agent ? 'Always allow via agent' : 'Always allow', title: agent ? 'Use the agent’s own permission scope' : 'Preview the saved permission rule' },
            { id: 'session', label: 'Allow for this chat', title: 'Stop asking about this until the chat ends' },
            { id: 'once', label: 'Allow', primary: true, title: 'Allow this one call', kbd: allowKey },
          ]}
          onChoose={(id: string) => {
            if (id === 'always' && !agent) setEditingRule(true);
            else if (id === 'deny') onDecide(false);
            else onDecide(true, id === 'always' ? 'always' : id === 'session' ? 'session' : 'once');
          }}
        />
      </div>
      {editingRule && <div className="space-y-2 rounded-lg border border-border bg-background/60 p-3">
        <p className="text-xs font-medium">Always allow · saved policy rules</p>
        <p className="text-[11px] text-muted-foreground">One rule per line. These exact patterns will be saved for future calls.</p>
        {approval.rulePreview?.error && <p role="alert" className="text-xs text-amber-600 dark:text-amber-400">{approval.rulePreview.error}</p>}
        {!approval.rulePreview && <p className="text-xs text-muted-foreground">Loading the exact policy rules…</p>}
        <textarea aria-label="Permission rule patterns" value={ruleDraft ?? approval.rulePreview?.rules.join('\n') ?? ''} onChange={event => setRuleDraft(event.target.value)} className="w-full rounded-md border border-border bg-background p-2 font-mono text-xs" rows={3} />
        <div className="flex justify-end gap-2"><button type="button" onClick={() => { setEditingRule(false); onRuleEditorCancel?.(); }} className="rounded-full px-4 py-2 text-xs font-medium hover:bg-secondary">Cancel</button><button type="button" disabled={!rules.length || !approval.rulePreview || (!!agent && !!approval.rulePreview.error)} onClick={() => onDecide(true, 'always', rules)} className="rounded-full bg-mira-blue px-4 py-2 text-xs font-medium text-mira-on-accent disabled:opacity-40">Save rule & allow</button></div>
      </div>}
      <div className="px-1.5 pb-1.5 text-right text-[10.5px] text-muted-foreground/60 touch:hidden">
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
