import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri runs dev on a fixed port and we don't want vite to fall back.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ["**/crates/**", "**/target/**"] },
  },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
  },
});
