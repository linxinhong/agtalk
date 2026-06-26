import { defineConfig } from 'wxt';
import react from '@vitejs/plugin-react';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = fileURLToPath(new URL('.', import.meta.url));

export default defineConfig({
  srcDir: 'src',
  entrypointsDir: '../entrypoints',
  publicDir: '../public',
  alias: {
    '@': path.resolve(__dirname, './src'),
  },
  vite: () => ({
    plugins: [react()],
  }),
  manifest: {
    name: 'agtalk Web Bridge',
    version: '0.1.0',
    description: 'Bridge ChatGPT / Claude / Sider web conversations to agtalk local agent bus',
    permissions: ['storage', 'activeTab', 'notifications', 'scripting'],
    host_permissions: [
      'https://chatgpt.com/*',
      'https://claude.ai/*',
      'https://sider.ai/*',
      'https://chatglm.cn/*',
      'http://127.0.0.1:19527/*',
    ],
    icons: {
      '16': 'icons/icon.svg',
      '32': 'icons/icon.svg',
      '48': 'icons/icon.svg',
      '128': 'icons/icon.svg',
    },
  },
});
