import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 约定：dev 端口 1420，且清屏关掉以便看 HMR。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: "./src/test-setup.ts",
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
