import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
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
