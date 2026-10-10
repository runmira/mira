import { SectionInput } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { getPreferredEditorId, listEditors } from '@/lib/editors';
import { isMacPlatform } from '@/lib/keybindings';
import { PREF_KEYS, useBoolPref, useStringPref } from '@/lib/prefs';
import { playTurnSound } from '@/lib/sound';
import {
  Bell,
  Cog,
  Globe2,
  MessagesSquare,
  PanelLeft,
  PenLine,
  SquareTerminal,
  Timer,
} from 'lucide-react';
import React, { useEffect, useState } from 'react';
import { getSettings, putSettings } from '../../api';
import type { Mode, SessionsSettings } from '../../types';
import { EditorIcon } from '../EditorPicker';
import { TRow, TSection, TSwitch } from './SettingsFields';
import { Draft } from './types';
export const MODES: Mode[] = ['plan', 'manual', 'auto', 'edit', 'yolo'];

export const MODE_DESCRIPTIONS: Record<Mode, string> = {
  plan: 'Plan first, then execute with approvals.',
  manual: 'Ask before every write, edit, or command.',
  auto: 'Auto-approve writes/edits; ask on bash.',
  edit: 'Auto-approve everything unless a rule blocks.',
  yolo: 'No gating — do whatever the model asks.',
};

/* ---------- section: general ---------- */

export function GeneralSection({
  draft,
  setDraft,
}: {
  draft: Draft;
  setDraft: (u: (d: Draft) => Draft) => void;
}) {
  const [sidebarOpen, setSidebarOpen] = useBoolPref(PREF_KEYS.sidebarOpen, true);
  const [terminalRestore, setTerminalRestore] = useBoolPref(PREF_KEYS.terminalRestore, true);
  const [cmdEnterSend, setCmdEnterSend] = useBoolPref(PREF_KEYS.composerCmdEnter, false);
  const [follow, setFollow] = useBoolPref(PREF_KEYS.transcriptFollow, true);
  const [turnStats, setTurnStats] = useBoolPref(PREF_KEYS.transcriptTurnStats, true);
  const [notifyTurnDone, setNotifyTurnDone] = useBoolPref(PREF_KEYS.notifyTurnDone, false);
  const [turnSound, setTurnSound] = useBoolPref(PREF_KEYS.turnSound, true);
  const [browserAutoOpen, setBrowserAutoOpen] = useBoolPref(PREF_KEYS.browserAutoOpen, true);
  // Server-side: these act with no window open, so they live in Mira's
  // config rather than this browser's storage.
  const [sessionsCfg, setSessionsCfg] = useState<SessionsSettings | null>(null);
  const [sessionsError, setSessionsError] = useState<string | null>(null);
  useEffect(() => {
    getSettings()
      .then((v) =>
        setSessionsCfg(
          v.sessions ?? { auto_resume_after_limit: false, keep_awake_while_running: false },
        ),
      )
      .catch((e) => setSessionsError(String((e as Error).message ?? e)));
  }, []);
  function setSessionsFlag(key: keyof SessionsSettings, value: boolean) {
    const prev = sessionsCfg;
    setSessionsCfg((c) => (c ? { ...c, [key]: value } : c));
    setSessionsError(null);
    putSettings({ sessions: { [key]: value } })
      .then((v) => v.sessions && setSessionsCfg(v.sessions))
      .catch((e) => {
        setSessionsCfg(prev);
        setSessionsError(String((e as Error).message ?? e));
      });
  }
  const [preferredEditor, setPreferredEditor] = useStringPref(PREF_KEYS.preferredEditor, '');
  const [editorOptions, setEditorOptions] = useState<
    { value: string; label: string; icon?: React.ReactNode }[]
  >([]);
  const [notifyState, setNotifyState] = useState(() =>
    typeof Notification === 'undefined' ? 'unsupported' : Notification.permission,
  );

  useEffect(() => {
    let cancelled = false;
    listEditors()
      .then((v) => {
        if (cancelled) return;
        const detected = new Set(v.editors.map((e) => e.id));
        setEditorOptions(
          v.all.map((e) => ({
            value: e.id,
            label: detected.has(e.id) ? e.name : `${e.name} (not detected)`,
            icon: <EditorIcon entry={e} />,
          })),
        );
        if (!getPreferredEditorId() && v.default_id) setPreferredEditor(v.default_id);
      })
      .catch(() => {
        /* offline / server down — row keeps its placeholder */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function setNotify(v: boolean) {
    if (v && typeof Notification !== 'undefined' && Notification.permission === 'default') {
      try {
        setNotifyState(await Notification.requestPermission());
      } catch {
        setNotifyState(Notification.permission);
      }
    }
    setNotifyTurnDone(v);
    if (typeof Notification !== 'undefined') setNotifyState(Notification.permission);
  }

  return (
    <div className="flex flex-col gap-2.5">
      <TSection
        icon={<Cog className="size-3.5" />}
        title="General"
        description="Defaults new sessions inherit. Override per-session via the composer chips."
      >
        <TRow
          title="Default mode"
          description={MODE_DESCRIPTIONS[draft.mode]}
          control={
            <Select<Mode>
              value={draft.mode}
              onChange={(v) => setDraft((d) => ({ ...d, mode: v }))}
              options={MODES.map((m) => ({
                value: m,
                label: m,
                hint: MODE_DESCRIPTIONS[m],
              }))}
              className="h-8 w-full text-[13px] sm:w-64"
            />
          }
        />

        <TRow
          title="Max tokens"
          description="Cap on tokens the model can emit per response. Leave blank for the provider default."
          control={
            <SectionInput
              type="number"
              value={draft.maxTokens}
              onChange={(e) => setDraft((d) => ({ ...d, maxTokens: e.target.value }))}
              min={1}
              placeholder="(provider default)"
              className="h-8 text-[13px] sm:w-64"
            />
          }
        />
      </TSection>

      <TSection
        icon={<PenLine className="size-3.5" />}
        title="Composer"
        description="How the message box sends. Stored in this browser only."
      >
        <TRow
          title="Send with ⌘/Ctrl+Enter"
          description={
            isMacPlatform()
              ? 'Plain Enter inserts a newline; ⌘+Enter sends. Off: Enter sends, Shift+Enter is the newline.'
              : 'Plain Enter inserts a newline; Ctrl+Enter sends. Off: Enter sends, Shift+Enter is the newline.'
          }
          control={
            <TSwitch
              checked={cmdEnterSend}
              onChange={setCmdEnterSend}
              label="Send with mod+Enter"
            />
          }
        />
      </TSection>

      <TSection
        icon={<MessagesSquare className="size-3.5" />}
        title="Transcript"
        description="How the conversation reads. Stored in this browser only."
      >
        <TRow
          title="Auto-scroll while streaming"
          description="Stick to the latest output as it arrives. Scroll up to pause and get a jump-to-latest button."
          control={
            <TSwitch checked={follow} onChange={setFollow} label="Auto-scroll while streaming" />
          }
        />

        <TRow
          title="Per-turn timing & cost"
          description="Show “worked for Xs · N in · M out · $Y” chips on finished turns and hover actions."
          control={
            <TSwitch checked={turnStats} onChange={setTurnStats} label="Per-turn timing and cost" />
          }
        />
      </TSection>

      <TSection
        icon={<Bell className="size-3.5" />}
        title="Notifications"
        description="System alerts from this browser. Stored in this browser only."
      >
        <TRow
          title="Notify when Mira finishes or needs you"
          description={
            notifyState === 'denied'
              ? 'Blocked — allow notifications for this site in your browser settings, then turn this back on.'
              : notifyState === 'unsupported'
                ? 'This browser does not support desktop notifications.'
                : 'A system alert when a chat finishes, asks for approval, or has a question — only while Mira is in the background.'
          }
          status={
            notifyTurnDone && notifyState === 'granted'
              ? 'On — you’ll hear from Mira while it’s in the background.'
              : undefined
          }
          control={
            <TSwitch
              checked={notifyTurnDone && notifyState !== 'denied' && notifyState !== 'unsupported'}
              onChange={(v) => void setNotify(v)}
              label="Notify when Mira finishes or needs you"
            />
          }
        />
        <TRow
          title="Sound when a reply finishes"
          description="A short chime when Mira or an agent finishes a turn."
          control={
            <>
              <button
                type="button"
                onClick={() => playTurnSound(true)}
                className="h-7 rounded-md px-2.5 text-[12px] text-muted-foreground transition-colors hover:bg-fg/[0.06] hover:text-foreground"
              >
                Play
              </button>
              <TSwitch
                checked={turnSound}
                onChange={setTurnSound}
                label="Sound when a reply finishes"
              />
            </>
          }
        />
      </TSection>

      <TSection
        icon={<Timer className="size-3.5" />}
        title="Running sessions"
        description="What chats do while they run on their own. Saved in Mira's config, so it applies even with this window closed."
      >
        <TRow
          title="Auto resume after a limit resets"
          description="When a chat stops on a usage limit, pick it up again automatically once the limit resets, instead of waiting for you to press Resume."
          control={
            <TSwitch
              checked={sessionsCfg?.auto_resume_after_limit ?? false}
              onChange={(v) => setSessionsFlag('auto_resume_after_limit', v)}
              label="Auto resume after a limit resets"
            />
          }
        />
        <TRow
          title="Keep the screen awake while a session runs"
          description="Stops the display and the computer from sleeping while any chat is working. Released as soon as nothing is running. macOS and Linux."
          control={
            <TSwitch
              checked={sessionsCfg?.keep_awake_while_running ?? false}
              onChange={(v) => setSessionsFlag('keep_awake_while_running', v)}
              label="Keep the screen awake while a session runs"
            />
          }
        />
        {sessionsError && <p className="px-1 text-[12px] text-destructive">{sessionsError}</p>}
      </TSection>

      <TSection
        icon={<Globe2 className="size-3.5" />}
        title="Browser"
        description="The browser Mira and its agents share. Stored in this browser only."
      >
        <TRow
          title="Show the browser when it's used"
          description="Open the browser pane as soon as Mira or an agent starts browsing, so you can watch and take over."
          control={
            <TSwitch
              checked={browserAutoOpen}
              onChange={setBrowserAutoOpen}
              label="Show the browser when it's used"
            />
          }
        />
      </TSection>

      <TSection
        icon={<SquareTerminal className="size-3.5" />}
        title="External editor"
        description="Where files and folders open. The toolbar picker and file viewer use this. Stored in this browser only."
      >
        <TRow
          title="Preferred editor"
          description="Detected editors are listed first. Anything else is tried anyway — the server reports if it isn't installed."
          control={
            <Select<string>
              value={preferredEditor}
              onChange={(v) => setPreferredEditor(v)}
              options={editorOptions}
              placeholder={editorOptions.length === 0 ? 'Loading…' : 'System default'}
              className="h-8 w-full text-[13px] sm:w-64"
            />
          }
        />
      </TSection>

      <TSection
        icon={<PanelLeft className="size-3.5" />}
        title="Startup"
        description="How the app shell looks when you open Mira. Stored in this browser only."
      >
        <TRow
          title="Show sidebar on startup"
          description="Keep the session list visible. Turn off for a full-width transcript."
          control={
            <TSwitch
              checked={sidebarOpen}
              onChange={setSidebarOpen}
              label="Show sidebar on startup"
            />
          }
        />

        <TRow
          title="Restore terminal on startup"
          description="Reopen the integrated terminal if it was open last time. Turn off to always start with it closed."
          control={
            <TSwitch
              checked={terminalRestore}
              onChange={setTerminalRestore}
              label="Restore terminal on startup"
            />
          }
        />
      </TSection>
    </div>
  );
}
