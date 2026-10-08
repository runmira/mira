import type { ImageAttachment } from '../types';
import { clearDraft, queueDraft, readDraft, flushDrafts } from './composerDrafts';
export type DraftFile = { path: string; content: string; bytes: number };
export type RichDraft = { text: string; attachments: DraftFile[]; images: ImageAttachment[] };
type Stored = RichDraft & { key: string; at: number; bytes: number };
const AGE = 14 * 86400_000;
const cache = new Map<string, RichDraft>();
const dirty = new Set<string>();
function remember(key: string, draft: RichDraft) {
  cache.delete(key); cache.set(key, draft);
  // Only evict saved, idle drafts. A failed save must never discard files.
  for (const candidate of cache.keys()) {
    if (cache.size <= 40) break;
    if (candidate === key || dirty.has(candidate) || loading.has(candidate) || pending.has(candidate)) continue;
    cache.delete(candidate); loaded.delete(candidate); revisions.delete(candidate); cleared.delete(candidate);
  }
}
const revisions = new Map<string, number>();
const loaded = new Set<string>();
const loading = new Map<string, Promise<RichDraft>>();
const cleared = new Set<string>();
const pending = new Map<string, ReturnType<typeof setTimeout>>();
let serial: Promise<unknown> = Promise.resolve();
let database: Promise<IDBDatabase> | undefined;
const empty = (): RichDraft => ({ text: '', attachments: [], images: [] });
export function cachedDraft(key: string): RichDraft { return cache.get(key) ?? { ...empty(), text: readDraft(key) }; }
function report(key: string, failed: boolean) {
  if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent('mira:draft-storage', { detail: { key, error: failed ? 'Draft recovery could not be saved. Keep this window open until you send your attachments.' : null } }));
}
function db(): Promise<IDBDatabase> {
  return database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('mira-composer-drafts', 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore('drafts', { keyPath: 'key' });
      request.result.createObjectStore('metadata', { keyPath: 'key' });
    };
    request.onsuccess = () => { request.result.onversionchange = () => { request.result.close(); database = undefined; }; resolve(request.result); };
    request.onerror = () => { database = undefined; reject(request.error ?? new Error('Draft storage unavailable')); };
    request.onblocked = () => { database = undefined; reject(new Error('Close older Mira windows to enable recovery')); };
  });
}
export function restoreRichDraft(key: string): Promise<RichDraft> {
  if (loaded.has(key)) return Promise.resolve(cachedDraft(key));
  const current = loading.get(key); if (current) return current;
  const promise = loadDraft(key).finally(() => loading.delete(key));
  loading.set(key, promise); return promise;
}
async function loadDraft(key: string): Promise<RichDraft> {
  const revision = revisions.get(key) ?? 0;
  const hadChanges = cache.has(key);
  let stored: Stored | undefined;
  try {
    const storage = await db();
    stored = await new Promise((resolve, reject) => {
      const request = storage.transaction('drafts', 'readonly').objectStore('drafts').get(key);
      request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    });
  } catch { report(key, true); }
  let draft = stored && Date.now() - stored.at < AGE && typeof stored.text === 'string' && Array.isArray(stored.attachments) && Array.isArray(stored.images)
    ? { text: stored.text, attachments: stored.attachments, images: stored.images } : cachedDraft(key);
  if (hadChanges || (revisions.get(key) ?? 0) !== revision) {
    const current = cachedDraft(key);
    draft = cleared.has(key) ? current : {
      text: current.text,
      attachments: [...new Map([...draft.attachments, ...current.attachments].map(file => [file.path, file])).values()],
      images: [...new Map([...draft.images, ...current.images].map(image => [image.media_type + image.data, image])).values()],
    };
  }
  loaded.add(key); remember(key, draft);
  return draft;
}
async function persist(key: string) {
  try {
    await restoreRichDraft(key);
    const draft = cachedDraft(key);
    const savedRevision = revisions.get(key);
    const bytes = new Blob([JSON.stringify(draft)]).size;
    if (bytes > 12 * 1024 * 1024) throw new Error('Draft exceeds recovery storage limit');
    const storage = await db();
    await new Promise<void>((resolve, reject) => {
      // Commit payload and small retention metadata together. Never load all
      // binary payloads on startup or write images to synchronous localStorage.
      const tx = storage.transaction(['drafts', 'metadata'], 'readwrite');
      const drafts = tx.objectStore('drafts'); const metadata = tx.objectStore('metadata');
      tx.oncomplete = () => resolve(); tx.onerror = tx.onabort = () => reject(tx.error ?? new Error('Draft save failed'));
      if (draft.text || draft.attachments.length || draft.images.length) { drafts.put({ ...draft, key, at: Date.now(), bytes }); metadata.put({ key, at: Date.now(), bytes }); }
      else { drafts.delete(key); metadata.delete(key); }
      const request = metadata.getAll();
      request.onsuccess = () => {
        const entries = (request.result as {key: string; at: number; bytes: number}[]).sort((a, b) => b.at - a.at);
        let total = 0; let count = 0;
        for (const entry of entries) {
          total += entry.bytes; count++;
          if (Date.now() - entry.at >= AGE || count > 40 || total > 32 * 1024 * 1024) { drafts.delete(entry.key); metadata.delete(entry.key); }
        }
      };
    });
    if (savedRevision === revisions.get(key)) dirty.delete(key);
    remember(key, cachedDraft(key));
    report(key, false);
  } catch { report(key, true); }
}
function flush(key: string) {
  const timer = pending.get(key); if (timer) clearTimeout(timer); pending.delete(key);
  serial = serial.catch(() => {}).then(() => persist(key));
}
export function changeRichDraft(key: string, draft: RichDraft, immediate = false) {
  dirty.add(key); remember(key, draft); revisions.set(key, (revisions.get(key) ?? 0) + 1);
  if (draft.text) queueDraft(key, draft.text); else clearDraft(key);
  const timer = pending.get(key); if (timer) clearTimeout(timer);
  if (immediate) flush(key); else pending.set(key, setTimeout(() => flush(key), 300));
}
export function clearRichDraft(key: string) { cleared.add(key); changeRichDraft(key, empty(), true); }
export async function flushRichDrafts(): Promise<void> { for (const key of [...pending.keys()]) flush(key); flushDrafts(); await serial; }
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', flushRichDrafts);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flushRichDrafts(); });
}
