import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  // Rust 側のエラーを Vite の画面消去で流さない。
  clearScreen: false,
  server: {
    // `tauri.conf.json` の devUrl と同じ。空いていなければ別のポートに逃げずに失敗させる。
    port: 1437,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
});
