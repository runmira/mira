/**
 * Which input the user last used: `pointer` or `keyboard`, on
 * `<html data-input>`.
 *
 * Menus, popovers and dialogs move focus to their first item when they
 * open so arrow keys work. WebKit (the desktop app's web view) counts that
 * programmatic focus as keyboard focus even after a mouse click, so the
 * first item lit up with a focus ring the user never asked for. The
 * stylesheet hides focus outlines while the last input was a pointer; a
 * keypress brings them straight back for keyboard users.
 */
export function installInputModality(): void {
  const root = document.documentElement;
  const set = (mode: 'pointer' | 'keyboard') => {
    if (root.dataset.input !== mode) root.dataset.input = mode;
  };
  set('pointer');
  window.addEventListener('pointerdown', () => set('pointer'), true);
  window.addEventListener(
    'keydown',
    (e) => {
      // Modifier taps alone (⌘ to see shortcuts, ⇧ while clicking) aren't navigation.
      if (['Meta', 'Control', 'Alt', 'Shift'].includes(e.key)) return;
      set('keyboard');
    },
    true,
  );
}
