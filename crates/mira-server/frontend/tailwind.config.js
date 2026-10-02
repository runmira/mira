/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        border:      'hsl(var(--border))',
        input:       'hsl(var(--input))',
        ring:        'hsl(var(--ring))',
        background:  'hsl(var(--background))',
        foreground:  'hsl(var(--foreground))',
        primary: {
          DEFAULT: 'hsl(var(--primary))',
          foreground: 'hsl(var(--primary-foreground))',
        },
        secondary: {
          DEFAULT: 'hsl(var(--secondary))',
          foreground: 'hsl(var(--secondary-foreground))',
        },
        destructive: {
          DEFAULT: 'hsl(var(--destructive))',
          foreground: 'hsl(var(--destructive-foreground))',
        },
        muted: {
          DEFAULT: 'hsl(var(--muted))',
          foreground: 'hsl(var(--muted-foreground))',
        },
        accent: {
          DEFAULT: 'hsl(var(--accent))',
          foreground: 'hsl(var(--accent-foreground))',
        },
        popover: {
          DEFAULT: 'hsl(var(--popover))',
          foreground: 'hsl(var(--popover-foreground))',
          border: 'hsl(var(--popover-border))',
        },
        card: {
          DEFAULT: 'hsl(var(--card))',
          foreground: 'hsl(var(--card-foreground))',
        },
        panel: 'hsl(var(--panel))',
        tooltip: {
          DEFAULT: 'hsl(var(--tooltip))',
          foreground: 'hsl(var(--tooltip-foreground))',
          tag: 'hsl(var(--tooltip-tag))',
          'tag-foreground': 'hsl(var(--tooltip-tag-foreground))',
        },
        // Diff stat colors (`DiffStatLabel` uses these).
        diff: {
          addition: '#98c379',
          deletion: '#e06c75',
        },
        // Mira-specific accents kept as flat values — we reach for these
        // for chat bubbles, code, reasoning blocks, syntax highlighting.
        mira: {
          bg:       'rgb(var(--mira-bg) / <alpha-value>)',
          sidebar:  'rgb(var(--mira-sidebar) / <alpha-value>)',
          elev1:    'rgb(var(--mira-elev1) / <alpha-value>)',
          elev2:    'rgb(var(--mira-elev2) / <alpha-value>)',
          border:   'rgb(var(--mira-border) / <alpha-value>)',
          user:     'rgb(var(--mira-user) / <alpha-value>)',
          tool:     'rgb(var(--mira-tool) / <alpha-value>)',
          error:    'rgb(var(--mira-error) / <alpha-value>)',
          cyan:     'rgb(var(--mira-cyan) / <alpha-value>)',
          purple:   'rgb(var(--mira-purple) / <alpha-value>)',
          blue:     'rgb(var(--mira-blue) / <alpha-value>)',
          'on-accent': 'rgb(var(--mira-on-accent) / <alpha-value>)',
        },
        // Theme-aware overlays (see styles.css): `fg` is the subtle
        // light-on-surface tint (white on dark, ink on light); `shade` the
        // recess/scrim tint, softened on light.
        fg:    'rgb(var(--fg-rgb) / <alpha-value>)',
        shade: 'rgb(var(--shade-rgb) / calc(<alpha-value> * var(--shade-k)))',
      },
      borderRadius: {
        lg: 'var(--radius)',
        md: 'calc(var(--radius) - 2px)',
        sm: 'calc(var(--radius) - 4px)',
      },
      fontFamily: {
        sans:  ['Inter', '-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'system-ui', 'sans-serif'],
        tight: ['Inter', '-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'system-ui', 'sans-serif'],
        mono:  ['ui-monospace', 'SFMono-Regular', 'SF Mono', 'Menlo', 'monospace'],
      },
      keyframes: {
        'accordion-down': {
          from: { height: '0' },
          to:   { height: 'var(--radix-accordion-content-height)' },
        },
        'accordion-up': {
          from: { height: 'var(--radix-accordion-content-height)' },
          to:   { height: '0' },
        },
        'fade-in': {
          from: { opacity: '0', transform: 'translateY(4px)' },
          to:   { opacity: '1', transform: 'translateY(0)' },
        },
        // Centered-dialog entrance. Must include the `-50%, -50%`
        // centering translate inside the keyframe because any `transform`
        // property in the animation completely overrides the element's
        // static `transform: translate(-50%, -50%)` — without this, the
        // dialog plays a `translateY(4px)`-only transform during
        // animation (top-left of the box lands at screen center →
        // bottom-right quadrant), then snaps back to true center when
        // the animation clears. That was the "opens bottom-right, janks
        // to middle" bug.
        'dialog-in': {
          from: { opacity: '0', transform: 'translate(-50%, calc(-50% + 8px)) scale(0.98)' },
          to:   { opacity: '1', transform: 'translate(-50%, -50%) scale(1)' },
        },
        'thinking-bounce': {
          '0%, 60%, 100%': { opacity: '0.25', transform: 'translateY(0)' },
          '30%':           { opacity: '1',    transform: 'translateY(-2px)' },
        },
        // Sweeping highlight for the "Thinking…" text — background-clipped
        // to the glyphs so a bright band travels through the letters.
        // Values chosen so the visible band lands ON the text at t=50%.
        'text-shimmer': {
          '0%':   { 'background-position': '200% 0' },
          '100%': { 'background-position': '-200% 0' },
        },
      },
      animation: {
        'accordion-down': 'accordion-down 0.14s ease-out',
        'accordion-up':   'accordion-up 0.14s ease-out',
        'fade-in':        'fade-in 0.14s ease-out',
        'dialog-in':      'dialog-in 0.14s ease-out',
        'text-shimmer':   'text-shimmer 2.4s linear infinite',
      },
    },
  },
  plugins: [],
};
