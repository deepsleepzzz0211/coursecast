import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Tauri dev server conventions: fixed port, no auto-open, no clear screen
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "chrome105",
    minify: "esbuild",
    sourcemap: false,
  },
});
