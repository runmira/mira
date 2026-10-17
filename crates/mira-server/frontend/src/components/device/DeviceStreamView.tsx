/**
 * A device's live screen. Drag to touch; click it, then type to send keys.
 * Only mounted for the visible device, so hidden tabs don't stream.
 */
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { LoaderCircle } from 'lucide-react';
import { cn } from '@/lib/utils';
import type { Device } from './api';
import { DeviceStream, type Button, type Screen, type StreamStatus } from './deviceStream';

export type DeviceStreamHandle = { button(b: Button): void; rotate(): void };

export const DeviceStreamView = forwardRef<DeviceStreamHandle, { device: Device; onInput?: (ready: boolean) => void }>(
  function DeviceStreamView({ device, onInput }, ref) {
    const canvas = useRef<HTMLCanvasElement | null>(null);
    const stream = useRef<DeviceStream | null>(null);
    const [status, setStatus] = useState<{ s: StreamStatus; detail?: string }>({ s: 'connecting' });
    const [mjpeg, setMjpeg] = useState<string | null>(null);
    const [screen, setScreen] = useState<Screen | null>(null);
    const [focused, setFocused] = useState(false);
    const onInputRef = useRef(onInput);
    onInputRef.current = onInput;

    useImperativeHandle(ref, () => ({
      button: (b) => stream.current?.button(b),
      rotate: () => stream.current?.rotate(),
    }), []);

    useEffect(() => {
      if (!canvas.current || !device.stream) return;
      const s = new DeviceStream(device.platform, device.stream, canvas.current, {
        onStatus: (st, detail) => setStatus({ s: st, detail }),
        onInput: (ready) => onInputRef.current?.(ready),
        onScreen: setScreen,
        onMjpeg: setMjpeg,
      });
      stream.current = s;
      s.start();
      // Pause while the window is hidden; a hidden tab still decodes.
      const vis = () => (document.hidden ? s.stop() : s.start());
      document.addEventListener('visibilitychange', vis);
      return () => {
        document.removeEventListener('visibilitychange', vis);
        s.stop();
        stream.current = null;
      };
    }, [device.platform, device.id, device.stream]);

    const point = (e: React.PointerEvent) => {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      return {
        x: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)),
        y: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)),
      };
    };
    const down = useRef(false);
    const touchProps = {
      onPointerDown: (e: React.PointerEvent) => {
        if (e.button !== 0) return;
        // Keep the drag when it leaves the screen; best effort, since a
        // pointer the browser didn't start (synthetic, some pens) throws.
        try { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); } catch { /* not capturable */ }
        down.current = true;
        const p = point(e);
        stream.current?.touch('begin', p.x, p.y);
      },
      onPointerMove: (e: React.PointerEvent) => {
        if (!down.current) return;
        const p = point(e);
        stream.current?.touch('move', p.x, p.y);
      },
      onPointerUp: (e: React.PointerEvent) => {
        if (!down.current) return;
        down.current = false;
        const p = point(e);
        stream.current?.touch('end', p.x, p.y);
      },
      onPointerCancel: (e: React.PointerEvent) => {
        if (!down.current) return;
        down.current = false;
        const p = point(e);
        stream.current?.touch('end', p.x, p.y);
      },
    };

    const landscape = screen ? screen.orientation.startsWith('landscape') : false;
    return (
      <div className="relative flex h-full min-h-0 w-full items-center justify-center p-3">
        <div
          tabIndex={0}
          role="application"
          aria-label={`${device.name} screen. Click and drag to touch; type to send keys.`}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={(e) => { if (!e.metaKey && stream.current?.key(e.nativeEvent, 'down')) e.preventDefault(); }}
          onKeyUp={(e) => { if (!e.metaKey && stream.current?.key(e.nativeEvent, 'up')) e.preventDefault(); }}
          className={cn(
            'relative max-h-full max-w-full cursor-pointer touch-none select-none overflow-hidden rounded-[22px] bg-black shadow-lg shadow-shade/20 ring-1 ring-black/10 outline-none dark:ring-white/10',
            focused && 'ring-2 ring-ring',
          )}
          style={{ aspectRatio: screen ? (landscape ? `${screen.height} / ${screen.width}` : `${screen.width} / ${screen.height}`) : '9 / 19.5', height: '100%' }}
          {...touchProps}
        >
          <canvas ref={canvas} className={cn('block h-full w-full object-contain', mjpeg && 'hidden')} />
          {mjpeg && <img src={mjpeg} alt="" draggable={false} className="block h-full w-full object-contain" />}
          {status.s !== 'live' && (
            <div role={status.s === 'error' ? 'alert' : 'status'} className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/60 px-4 text-center text-[12px] text-white/85">
              {status.s === 'connecting' && <LoaderCircle className="size-4 animate-spin" />}
              {status.detail ?? (status.s === 'connecting' ? 'Connecting to the device…' : 'The stream stopped.')}
            </div>
          )}
        </div>
      </div>
    );
  },
);
