import { defineConfig } from 'wxt';
import vue from '@vitejs/plugin-vue';

export default defineConfig({
  srcDir: 'src',
  extensionApi: 'chrome',
  manifest: {
    name: 'agtalk',
    version: '0.1.0',
    permissions: ['storage'],
    host_permissions: ['http://127.0.0.1:19527/*'],
  },
  vite: () => ({
    plugins: [vue()],
  }),
});
