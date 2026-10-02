/** The chime when a reply finishes, unless it's turned off in Settings. */
import { getBoolPref, PREF_KEYS } from './prefs';

/** `force` plays it even when the setting is off (Settings' preview). */
export function playTurnSound(force = false): void {
  if (!force && !getBoolPref(PREF_KEYS.turnSound, true)) return;
  try {
    const audio = new Audio('/ping.mp3');
    audio.volume = 0.7;
    void audio.play().catch(() => {});
  } catch {
    /* audio unavailable */
  }
}
