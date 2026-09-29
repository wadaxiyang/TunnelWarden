import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { strictPort: true, port: 1420, watch: { ignored: ['**/target/**', '**/.upstream/**', '**/upstream/**', '**/src-tauri/**'] } }
});
