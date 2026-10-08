import { useMemo } from 'react';
import qrcode from 'qrcode-generator';

/**
 * A QR code drawn as SVG: rounded modules and soft finder squares instead
 * of a hard pixel grid. Dark on light regardless of theme — scanners want
 * contrast, and a light tile reads as "scan me" in dark mode too.
 */
export function QrCode({ value, size = 184 }: { value: string; size?: number }) {
  const { count, dots } = useMemo(() => {
    const qr = qrcode(0, 'M');
    qr.addData(value);
    qr.make();
    const count = qr.getModuleCount();
    const dots: [number, number][] = [];
    for (let r = 0; r < count; r++) {
      for (let c = 0; c < count; c++) {
        if (qr.isDark(r, c) && !inFinder(r, c, count)) dots.push([r, c]);
      }
    }
    return { count, dots };
  }, [value]);

  const quiet = 2;
  const box = count + quiet * 2;
  const finders: [number, number][] = [
    [0, 0],
    [0, count - 7],
    [count - 7, 0],
  ];

  return (
    <svg
      viewBox={`0 0 ${box} ${box}`}
      width={size}
      height={size}
      role="img"
      aria-label="QR code to pair a device"
      className="rounded-2xl bg-white shadow-[0_8px_30px_-12px_rgba(0,0,0,0.45)]"
    >
      <g transform={`translate(${quiet} ${quiet})`} fill="#0b0b0f">
        {dots.map(([r, c]) => (
          <rect key={`${r}-${c}`} x={c + 0.08} y={r + 0.08} width={0.84} height={0.84} rx={0.32} />
        ))}
        {finders.map(([r, c]) => (
          <g key={`f-${r}-${c}`} transform={`translate(${c} ${r})`}>
            <rect x={0.5} y={0.5} width={6} height={6} rx={1.9} fill="none" stroke="#0b0b0f" strokeWidth={1} />
            <rect x={2} y={2} width={3} height={3} rx={0.9} />
          </g>
        ))}
      </g>
    </svg>
  );
}

function inFinder(r: number, c: number, n: number): boolean {
  const near = (v: number, start: number) => v >= start && v < start + 7;
  return (near(r, 0) && near(c, 0)) || (near(r, 0) && near(c, n - 7)) || (near(r, n - 7) && near(c, 0));
}
