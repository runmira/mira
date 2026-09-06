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
        },
        card: {
          DEFAULT: 'hsl(var(--card))',
          foreground: 'hsl(var(--card-foreground))',
        },
        // Mira-specific accents kept as flat values — we reach for these
        // for chat bubbles, code, reasoning blocks, syntax highlighting.
        mira: {
          bg:       '#131417',
          sidebar:  '#1a1c20',
          elev1:    '#212327',
          elev2:    '#2a2d33',
          border:   '#2b2e34',
          user:     '#a6d189',
          tool:     '#e5c07b',
          error:    '#f7768e',
          cyan:     '#7dcfff',
          purple:   '#bb9af7',
          blue:     '#7aa2f7',
        },
      },
      borderRadius: {
        lg: 'var(--radius)',
        md: 'calc(var(--radius) - 2px)',
        sm: 'calc(var(--radius) - 4px)',
      },
      fontFamily: {
        mono: ['ui-monospace', 'SFMono-Regular', 'SF Mono', 'Menlo', 'monospace'],
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
        'thinking-bounce': {
          '0%, 60%, 100%': { opacity: '0.25', transform: 'translateY(0)' },
          '30%':           { opacity: '1',    transform: 'translateY(-2px)' },
        },
      },
      animation: {
        'accordion-down': 'accordion-down 0.14s ease-out',
        'accordion-up':   'accordion-up 0.14s ease-out',
        'fade-in':        'fade-in 0.14s ease-out',
      },
    },
  },
  plugins: [],
};
