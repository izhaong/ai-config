import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite 配 Tauri 2 + React 19
// dev server 端口固定 5173(tauri.conf.json devUrl 对齐)
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
  },
  build: {
    target: "esnext",
    outDir: "dist",
    emptyOutDir: true,
  },
});
