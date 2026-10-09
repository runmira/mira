import {
  ATTACH_FILE_EVENT,
  COMPOSE_QUOTE_EVENT,
  COMPOSE_TEXT_EVENT,
  registerAttachmentIntake,
  type ComposerQuote,
} from '@/lib/attachBridge';
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { readFile } from '../../api';
import { mapPosturesToModes, MIRA_MODE_TO_POSTURE } from '../../lib/agentPostures';
import { draftKey } from '../../lib/composerDrafts';
import { useIsPhone } from '../../lib/mobile';
import { PREF_KEYS, useBoolPref } from '../../lib/prefs';
import {
  cachedDraft,
  changeRichDraft,
  clearRichDraft,
  restoreRichDraft,
  type RichDraft,
} from '../../lib/richComposerDrafts';
import type { Mode } from '../../types';
import { type ComposerNotice } from '../ComposerNoticeStack';
import { type MentionInputHandle } from '../MentionInput';
import { quoteMarkdown } from '../QuoteCard';
import {
  customToCommand,
  filterCommands,
  groupCommands,
  slashState,
  type PaletteGroup,
  type SlashCommand,
} from '../commands';
import { MODES } from '../composer/ComposerMenus';
import {
  formatBytes,
  looksBinary,
  NATIVE_ATTACH_MAX_BYTES,
  readImage,
  renderAttachments,
} from '../composer/attachments';
import { Attachment, ImageData, Props } from '../composer/types';
export function useComposerController({
  disabled,
  busy,
  mode,
  cwd,
  onSend,
  onQueueMessage,
  onSetMode,
  onSetModel,
  onAcpModes,
  onAcpCurrentMode,
  onPickAgentMode,
  agentDriving,
  onOpenPicker,
  onNewChat,
  onOpenSettings,
  onRunReview,
  onSetGoal,
  onClearGoal,
  onCompact,
  onRemember,
  onUndo,
  engine,
  sessionId,
  skills,
  commands,
  pendingApproval: fallbackApproval,
  pendingApprovals,
  ruleEditorCallId,
  onActiveApprovalChange,
  pendingPlans,
  pendingQuestions,
  notices = [],
  pendingPlan: fallbackPlan,
  pendingAskUser: fallbackQuestion,
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
      current:
        (current
          ? (Object.keys(MIRA_MODE_TO_POSTURE) as Mode[]).find(
              (m) => MIRA_MODE_TO_POSTURE[m] === current.posture.key,
            )
          : null) ?? null,
    };
  }, [agentDriving, acpModes_modes, acpCurrentMode, onPickAgentMode, engine?.driver]);

  const phone = useIsPhone();

  const [text, setText] = useState(() => cachedDraft(draftKey(sessionId)).text);

  const [attachments, setAttachmentState] = useState<Attachment[]>(
    () => cachedDraft(draftKey(sessionId)).attachments,
  );

  const [images, setImageState] = useState<ImageData[]>(
    () => cachedDraft(draftKey(sessionId)).images,
  );

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
      if (textOwner.current === owner)
        setAttachError(`couldn't read image: ${(e as Error).message}`);
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

  function setAttachments(
    value: Attachment[] | ((prev: Attachment[]) => Attachment[]),
    key = textOwner.current,
  ) {
    const previous = key === textOwner.current ? draftState.current : cachedDraft(key);
    const next = {
      ...previous,
      attachments: typeof value === 'function' ? value(previous.attachments) : value,
    };
    applyDraft(key, next, true);
    if (textOwner.current === key) setAttachmentState(next.attachments);
  }

  function setImages(
    value: ImageData[] | ((prev: ImageData[]) => ImageData[]),
    key = textOwner.current,
  ) {
    const previous = key === textOwner.current ? draftState.current : cachedDraft(key);
    const next = {
      ...previous,
      images: typeof value === 'function' ? value(previous.images) : value,
    };
    applyDraft(key, next, true);
    if (textOwner.current === key) setImageState(next.images);
  }

  useLayoutEffect(() => {
    let alive = true;
    textOwner.current = draftKeyFor;
    const local = cachedDraft(draftKeyFor);
    draftState.current = local;
    setText(local.text);
    setAttachmentState(local.attachments);
    setImageState(local.images);
    restoringDraft.current = true;
    mentionRef.current?.setText(local.text);
    restoringDraft.current = false;
    setDraftError(null);
    setAttachError(null);
    setAttachLoading(false);
    setPreview(null);
    void restoreRichDraft(draftKeyFor).then((stored) => {
      if (!alive || textOwner.current !== draftKeyFor) return;
      draftState.current = stored;
      setText(stored.text);
      setAttachmentState(stored.attachments);
      setImageState(stored.images);
      restoringDraft.current = true;
      mentionRef.current?.setText(stored.text);
      restoringDraft.current = false;
    });
    return () => {
      alive = false;
    };
  }, [draftKeyFor]);

  useEffect(() => {
    const listener = (event: Event) => {
      const detail = (event as CustomEvent<{ key: string; error: string | null }>).detail;
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
    const found = filterCommands(
      (slash as { query: string }).query,
      paletteTrigger,
      skills,
      customCmds,
    );
    return paletteTrigger === '@'
      ? [{ key: 'all', label: '', items: found } as PaletteGroup]
      : groupCommands(found);
  }, [paletteVisible, slash, paletteTrigger, skills, customCmds]);

  const paletteMatches = useMemo(() => paletteGroups.flatMap((g) => g.items), [paletteGroups]);

  useEffect(() => {
    setSlashIdx(0);
  }, [text]);

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
    [
      onNewChat,
      onSetMode,
      onSetModel,
      onOpenPicker,
      onOpenSettings,
      onRunReview,
      onSetGoal,
      onClearGoal,
      onCompact,
      onRemember,
      onUndo,
      onSend,
    ],
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
    const prefix =
      slash.mode === 'palette' || slash.mode === 'args' ? text.slice(0, slash.triggerStart) : '';
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
      setAttachments(
        (prev) => [
          ...prev.filter((a) => a.path !== f.path),
          { path: f.path, content: f.content, bytes: f.bytes },
        ],
        owner,
      );
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
      if (
        /^image\/(png|jpeg|gif|webp)$/.test(file.type) ||
        /\.(png|jpe?g|gif|webp)$/i.test(file.name)
      ) {
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
      setAttachments(
        (prev) => [
          // De-dupe by filename (client-side files have no path).
          ...prev.filter((a) => a.path !== file.name),
          { path: file.name, content, bytes: file.size },
        ],
        owner,
      );
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
      event.preventDefault();
      event.dataTransfer.dropEffect = 'copy';
      setDragging(true);
    };
    const drop = (event: DragEvent) => {
      if (!event.dataTransfer?.files.length) return;
      event.preventDefault();
      setDragging(false);
      void attachNativeFiles(event.dataTransfer.files);
    };
    const leave = (event: DragEvent) => {
      if (!event.relatedTarget) setDragging(false);
    };
    window.addEventListener('dragover', over);
    window.addEventListener('drop', drop);
    window.addEventListener('dragleave', leave);
    window.addEventListener('dragend', leave);
    return () => {
      window.removeEventListener('dragover', over);
      window.removeEventListener('drop', drop);
      window.removeEventListener('dragleave', leave);
      window.removeEventListener('dragend', leave);
    };
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
    const unregister = registerAttachmentIntake((file) => {
      void attachNativeFile(file);
    });
    window.addEventListener(ATTACH_FILE_EVENT, onAttach);
    return () => {
      unregister();
      window.removeEventListener(ATTACH_FILE_EVENT, onAttach);
    };
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
    if (
      (!trimmed && attachments.length === 0 && images.length === 0 && quotes.length === 0) ||
      disabled
    )
      return;

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
    const body =
      attachments.length > 0 ? renderAttachments(attachments, cwd) + '\n\n' + quoted : quoted;
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
    ...questions.map((item) => ({
      id: `question:${item.callId}`,
      kind: 'question' as const,
      title: 'Answer a question',
      detail: item.proposal.questions[0]?.question,
    })),
    ...plans.map((item) => ({
      id: `plan:${item.callId}`,
      kind: 'plan' as const,
      title: 'Review the proposed plan',
    })),
    ...approvals.map((item) => ({
      id: `approval:${item.callId}`,
      kind: 'approval' as const,
      title: 'Permission requested',
      detail: item.call.function.name,
    })),
    ...notices,
  ];

  const [selectedNotice, setSelectedNotice] = useState<string | null>(null);

  useEffect(() => {
    if (ruleEditorCallId) setSelectedNotice(`approval:${ruleEditorCallId}`);
  }, [ruleEditorCallId]);

  const selected = noticeItems.find((item) => item.id === selectedNotice) ?? noticeItems[0];

  const pendingAskUser = questions.find((item) => selected?.id === `question:${item.callId}`);

  const pendingPlan = plans.find((item) => selected?.id === `plan:${item.callId}`);

  const pendingApproval = approvals.find((item) => selected?.id === `approval:${item.callId}`);

  const activePromptKind = pendingAskUser
    ? 'ask_user'
    : pendingPlan
      ? 'plan'
      : pendingApproval
        ? 'approval'
        : null;

  useEffect(() => {
    onActiveApprovalChange?.(pendingApproval?.callId ?? null);
    return () => onActiveApprovalChange?.(null);
  }, [pendingApproval?.callId, onActiveApprovalChange]);
  return {
    draftError,
    dragging,
    submit,
    setDragging,
    attachNativeFiles,
    selected,
    noticeItems,
    setSelectedNotice,
    preview,
    setPreview,
    planActive,
    goalComposing,
    togglePlan,
    setGoalComposing,
    activePromptKind,
    images,
    setImages,
    quotes,
    setQuotes,
    attachments,
    attachError,
    removeAttachment,
    plans,
    pendingPlan,
    questions,
    pendingAskUser,
    mentionRef,
    addImages,
    text,
    setText,
    restoringDraft,
    persistText,
    setSlashFeedback,
    paletteVisible,
    paletteMatches,
    setSlashIdx,
    slashIdx,
    commitPaletteChoice,
    slash,
    updateText,
    paletteTrigger,
    cmdEnterSend,
    phone,
    paletteGroups,
    slashFeedback,
    approvals,
    pendingApproval,
    openNativeFiles,
    attachLoading,
    textOwner,
    setAttachLoading,
    setAttachError,
    attachNativeFile,
    modelPopOpen,
    setModelPopOpen,
    agentPicker,
    modeLabel,
    filePickerOpen,
    setFilePickerOpen,
    attachFile,
    fileInputRef,
  };
}
