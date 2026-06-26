import type { Config } from 'tailwindcss';

export default {
  content: ['./entrypoints/**/*.html', './entrypoints/**/*.tsx', './src/**/*.tsx'],
  theme: {
    extend: {},
  },
  plugins: [],
} satisfies Config;
