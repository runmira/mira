import type { KeyboardEvent } from 'react';
/** Move between the engine rail and model search without changing the selected model. */
export function navigateEnginePicker(event: KeyboardEvent<HTMLElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const root = event.currentTarget;
  const target = event.target as HTMLElement;
  const input = root.querySelector<HTMLInputElement>('input[cmdk-input]');
  const rows = [...root.querySelectorAll<HTMLButtonElement>('[data-engine-row]')];
  let next: HTMLElement | null | undefined;
  if (target === input && input && ((event.key === 'ArrowLeft' && !event.shiftKey && !input.value) || (event.key === 'Tab' && event.shiftKey))) {
    next = rows.find(row => row.getAttribute('aria-pressed') === 'true') ?? rows[0];
  } else if (rows.includes(target as HTMLButtonElement)) {
    if (event.key === 'ArrowRight') next = input;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      const index = rows.indexOf(target as HTMLButtonElement);
      next = rows[(index + (event.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length];
    }
  }
  if (next) {
    event.preventDefault(); event.stopPropagation(); next.focus({preventScroll:true});
  }
}
