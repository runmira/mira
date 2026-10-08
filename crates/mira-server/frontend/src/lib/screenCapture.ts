import { captureNativeScreenshot } from './desktop';
import type { ImageSource } from '../types';
export async function captureScreenshot(): Promise<File | null> {
  const native = await captureNativeScreenshot();
  if (native !== undefined) {
    if (native) Object.assign(native, { source: { name: 'Selected screen region', captured_at: new Date().toISOString() } satisfies ImageSource });
    return native;
  }
  if (!navigator.mediaDevices?.getDisplayMedia) throw new Error('Screen capture is unavailable here. Attach or drop a screenshot instead.');
  const stream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
  try {
    const video = document.createElement('video'); video.muted = true; video.srcObject = stream;
    await video.play();
    const track = stream.getVideoTracks()[0];
    const canvas = document.createElement('canvas'); canvas.width = video.videoWidth; canvas.height = video.videoHeight;
    if (!canvas.width || !canvas.height) throw new Error('The selected screen has no image');
    canvas.getContext('2d')!.drawImage(video, 0, 0);
    const blob = await new Promise<Blob>((resolve, reject) => canvas.toBlob(blob => blob ? resolve(blob) : reject(new Error('Could not encode screenshot')), 'image/png'));
    const file = new File([blob], 'Screenshot.png', { type: 'image/png' });
    Object.assign(file, { source: { name: 'Screenshot', window: track.label || undefined, captured_at: new Date().toISOString(), width: canvas.width, height: canvas.height } satisfies ImageSource });
    return file;
  } finally { stream.getTracks().forEach(track => track.stop()); }
}
