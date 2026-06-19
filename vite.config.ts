import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [vue()],
  base: "./",
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 5174 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
}));
