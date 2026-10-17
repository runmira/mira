/**
 * Asking the Device pane to show a device (an agent opened it). The pane
 * may not be mounted yet, so the latest request is also kept until the
 * pane takes it.
 */
const EVENT = 'mira:device-focus';
let pending: string | null = null;

export function focusDevice(device: string) {
  pending = device;
  window.dispatchEvent(new CustomEvent(EVENT, { detail: device }));
}

/** The request waiting for a pane that wasn't open, once. */
export function takePendingFocus(): string | null {
  const p = pending;
  pending = null;
  return p;
}

export function onDeviceFocus(handler: (device: string) => void): () => void {
  const listener = (e: Event) => handler((e as CustomEvent<string>).detail);
  window.addEventListener(EVENT, listener);
  return () => window.removeEventListener(EVENT, listener);
}
