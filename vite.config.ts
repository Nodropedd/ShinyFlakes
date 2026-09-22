import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri drives the dev server on a fixed port and expects a hard failure
// rather than a silent fallback if that port is taken.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    // chrome105 is right for the desktop webviews and wrong for a phone.
    // Android's WebView is updated through the Play Store, so on a device
    // that is old, has no Play Services, or ships a vendor ROM it can sit
    // years behind — and syntax it cannot parse means the bundle never
    // executes, which shows up as a blank near-black window rather than an
    // error. safari13 is the conservative floor Tauri recommends for mobile.
    target:
      process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    sourcemap: false,
    minify: "esbuild",
  },
});
