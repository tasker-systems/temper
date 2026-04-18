/**
 * Temper Design System — Tailwind preset
 *
 * Import into the production SvelteKit app's tailwind.config.js as a preset,
 * OR copy the @theme block into a CSS file for Tailwind v4 projects.
 * Mirrors _source/app.css so production code keeps parity with this system.
 *
 * Usage (Tailwind v3):
 *   module.exports = { presets: [require('./tailwind.config.preset.js')], ... };
 *
 * Usage (Tailwind v4, CSS-first):
 *   See ./tailwind.theme.css next to this file.
 */

module.exports = {
  theme: {
    extend: {
      colors: {
        // Ground & text
        obsidian:  { DEFAULT: '#0a0a0f', 2: '#0c0c11', 3: '#12121a' },
        parchment: '#e8e4df',

        // Primary accent scale — the temper-* scale from production app.css
        temper: {
          50:  '#f0f7ff', 100: '#e0effe', 200: '#bae0fd', 300: '#7ccbfc',
          400: '#36b2f8', 500: '#0c99e9', 600: '#0079c7', 700: '#0060a1',
          800: '#045185', 900: '#09446e', 950: '#062b49',
          // Marketing accent (hand-tuned, not on the 50→950 scale)
          blue: '#7eb8da',
        },

        // Diagram-only semantic palette
        session:  { light: '#86efac', dark: '#166534', mid: '#82c99a' },
        decision: { light: '#fcd34d', dark: '#92400e' },
        deferred: { light: '#94a3b8', dark: '#475569' },
        rot:      { light: '#fca5a5', dark: '#991b1b' },

        // Graph node palette from lib/graph/styling.ts
        graph: {
          research: '#7eb8da',
          task:     '#f0a870',
          session:  '#82c99a',
          concept:  '#d48ac7',
        },
      },

      fontFamily: {
        serif: ['"Source Serif 4"', '"Source Serif Pro"', 'Georgia', 'Times New Roman', 'serif'],
        mono:  ['"JetBrains Mono"', '"Fira Code"', 'ui-monospace', 'monospace'],
      },

      letterSpacing: {
        'label':   '0.20em',
        'strip':   '0.22em',
        'eyebrow': '0.28em',
        'ui':      '0.05em',
        'mark':    '0.15em',
      },

      maxWidth: {
        'column': '52rem',   // editorial long-form
        'wide':   '50rem',   // landing
      },

      borderColor: {
        rule:    'rgba(255,255,255,0.06)',
        'rule-2':'rgba(255,255,255,0.10)',
      },

      transitionDuration: { DEFAULT: '150ms' },
      transitionTimingFunction: { DEFAULT: 'ease' },
    },
  },

  // No plugins — Temper intentionally avoids @tailwindcss/forms, typography, etc.
  // Editorial prose styles live in colors_and_type.css; form chrome is hand-built.
  plugins: [],
};
