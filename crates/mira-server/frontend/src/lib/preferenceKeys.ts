export const PREF_KEYS = {
  sidebarOpen: 'mira.sidebar.open',
  terminalRestore: 'mira.terminal.restore',
  composerCmdEnter: 'mira.composer.cmdEnter',
  transcriptFollow: 'mira.transcript.follow',
  transcriptTurnStats: 'mira.transcript.turnStats',
  diffLayout: 'mira.diff.layout',
  reduceMotion: 'mira.reduceMotion',
  notifyTurnDone: 'mira.notify.turnDone',
  browserAutoOpen: 'mira.browser.autoOpen',
  preferredEditor: 'mira.editor.preferred',
  /** 'system' | 'light' | 'dark' — see lib/theme.ts. */
  theme: 'mira.theme',
  /** Play a sound when a reply finishes. */
  turnSound: 'mira.sound.turnDone',
  /** Accent color and interface size — see lib/appearance.ts. */
  accent: 'mira.accent',
  uiScale: 'mira.uiScale',
  /** Diff appearance — see lib/diffPrefs.ts. */
  diffWrap: 'mira.diff.wrap',
  diffInline: 'mira.diff.inline',
  diffMarkers: 'mira.diff.markers',
  diffLineNumbers: 'mira.diff.lineNumbers',
  diffTint: 'mira.diff.tint',
  /** Work tab shelves the user folded, comma-separated (Done by default). */
  workShelvesClosed: 'mira.work.shelvesClosed',
} as const;
