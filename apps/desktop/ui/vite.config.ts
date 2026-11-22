import tailwindcss from "@tailwindcss/vite";
import solid from "vite-plugin-solid";
import { defineConfig } from "vitest/config";

// Tauri expects a fixed dev port and must not clear its output.
export default defineConfig({
  plugins: [solid(), tailwindcss()],
  clearScreen: false,
  // TAURI_ENV_DEBUG lets debug app builds enable dev-only instrumentation (perf monitor).
  envPrefix: ["VITE_", "TAURI_ENV_"],
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "es2022" },
  test: { environment: "jsdom" },
  resolve: { conditions: ["development", "browser"] },
});
