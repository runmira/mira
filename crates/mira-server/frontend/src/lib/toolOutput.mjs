const ANSI_ESCAPE =
  /\x1b(?:\[[0-?]*[ -/]*[@-~]|\][\s\S]*?(?:\x07|\x1b\\)|[PX^_][\s\S]*?\x1b\\|[@-_])/g;

const ANSI_COLORS = [
  '#000000',
  '#cd3131',
  '#0dbc79',
  '#e5e510',
  '#2472c8',
  '#bc3fbc',
  '#11a8cd',
  '#e5e5e5',
  '#666666',
  '#f14c4c',
  '#23d18b',
  '#f5f543',
  '#3b8eea',
  '#d670d6',
  '#29b8db',
  '#ffffff',
];

function color256(index) {
  if (index < 16) return ANSI_COLORS[index];
  if (index < 232) {
    const value = index - 16;
    const channel = (part) => (part === 0 ? 0 : 55 + part * 40);
    return `rgb(${channel(Math.floor(value / 36))}, ${channel(Math.floor(value / 6) % 6)}, ${channel(value % 6)})`;
  }
  const gray = 8 + (index - 232) * 10;
  return `rgb(${gray}, ${gray}, ${gray})`;
}

function applySgr(style, parameters) {
  const codes = parameters.split(/[;:]/).map((value) => Number(value || 0));
  for (let i = 0; i < codes.length; i++) {
    const code = codes[i];
    if (code === 0) {
      for (const key of Object.keys(style)) delete style[key];
    } else if (code === 1) style.fontWeight = 'bold';
    else if (code === 2) style.opacity = '0.7';
    else if (code === 3) style.fontStyle = 'italic';
    else if (code === 4) style.textDecoration = 'underline';
    else if (code === 7) style.filter = 'invert(1)';
    else if (code === 9) style.textDecoration = 'line-through';
    else if (code === 22) {
      delete style.fontWeight;
      delete style.opacity;
    } else if (code === 23) delete style.fontStyle;
    else if (code === 24 || code === 29) delete style.textDecoration;
    else if (code === 27) delete style.filter;
    else if (code === 39) delete style.color;
    else if (code === 49) delete style.backgroundColor;
    else if ((code >= 30 && code <= 37) || (code >= 90 && code <= 97)) {
      style.color = ANSI_COLORS[code >= 90 ? code - 90 + 8 : code - 30];
    } else if ((code >= 40 && code <= 47) || (code >= 100 && code <= 107)) {
      style.backgroundColor = ANSI_COLORS[code >= 100 ? code - 100 + 8 : code - 40];
    } else if ((code === 38 || code === 48) && codes[i + 1] === 5) {
      const color = color256(Math.max(0, Math.min(255, codes[i + 2] ?? 0)));
      style[code === 38 ? 'color' : 'backgroundColor'] = color;
      i += 2;
    } else if ((code === 38 || code === 48) && codes[i + 1] === 2) {
      const channels = codes.slice(i + 2, i + 5).map((part) => Math.max(0, Math.min(255, part)));
      if (channels.length === 3) {
        style[code === 38 ? 'color' : 'backgroundColor'] = `rgb(${channels.join(', ')})`;
        i += 4;
      }
    }
  }
}

export function ansiToSegments(input) {
  const text = input.replace(ANSI_ESCAPE, '').replace(/\r\n?/g, '\n').replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g, '');
  const segments = [];
  let style = {};
  let plain = '';

  const flush = () => {
    if (!plain) return;
    segments.push({ text: plain, style: { ...style } });
    plain = '';
  };

  const sgr = /\x1b\[([0-?]*[ -/]*)([@-~])/g;
  let cursor = 0;
  let match;
  while ((match = sgr.exec(input)) !== null) {
    const prefix = input.slice(cursor, match.index);
    plain += prefix.replace(ANSI_ESCAPE, '').replace(/\r\n?/g, '\n').replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g, '');
    if (match[2] === 'm') {
      flush();
      applySgr(style, match[1]);
    }
    cursor = sgr.lastIndex;
  }
  plain += input.slice(cursor).replace(ANSI_ESCAPE, '').replace(/\r\n?/g, '\n').replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g, '');
  flush();

  return segments.length ? segments : [{ text, style: {} }];
}

export function parseBashOutput(content) {
  const match = /^exit=(-?\d+|unknown)\r?\n(?:(\(command timed out\))\r?\n)?--- output ---\r?\n/.exec(content);
  if (!match) return { output: content, exitCode: null, timedOut: false };

  const exitCode = match[1] === 'unknown' ? null : Number(match[1]);
  const body = content.slice(match[0].length);
  const hint = body.indexOf('\n--- hint ---\n');
  return {
    output: hint === -1 ? body : body.slice(0, hint),
    exitCode,
    timedOut: Boolean(match[2]),
  };
}

export function foldOutputLines(content, limit = 50, context = 10) {
  const lines = content.split('\n');
  if (lines.length <= limit) return { lines, hidden: 0 };

  const visible = [...lines.slice(0, context), ...lines.slice(-context)];
  return { lines: visible, hidden: lines.length - visible.length };
}
