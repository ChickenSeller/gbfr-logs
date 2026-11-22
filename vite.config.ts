import react from "@vitejs/plugin-react";
import path from "path";
import { defineConfig } from "vite";

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],
  build: {
    target: "es2022",
  },

  resolve: {
    alias: [{ find: "@", replacement: path.resolve(__dirname, "src") }],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent vite from obscuring rust errors
  clearScreen: false,

  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // 3. tell vite to ignore watching Rust sources and the Cargo workspace `target` dir
      // (watching locked build artifacts there crashes Vite with EBUSY on Windows)
      ignored: ["**/src-tauri/**", "**/src-hook/**", "**/protocol/**", "**/target/**"],
    },
  },

  test: {
    global: true,
    environment: "jsdom",
  },
}));
