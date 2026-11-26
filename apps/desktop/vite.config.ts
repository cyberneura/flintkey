import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  build: {
    rollupOptions: {
      // The launcher window and the settings window are separate pages.
      input: { launcher: 'index.html', settings: 'settings.html' },
    },
  },
  // Keep Vite from clearing the screen and hiding Rust errors.
  clearScreen: false,
  server: {
    // Same as devUrl in `tauri.conf.json`. Fail instead of moving to another port if it is taken.
    port: 1437,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
});
