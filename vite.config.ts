import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  // Rust errors are the ones worth reading during a plugin build; clearing the
  // screen is what hides them.
  clearScreen: false,
  server: {
    // Fixed, so the dev server is at a known address whatever else is running.
    port: 1420,
    strictPort: true,
    watch: {
      // The Cargo workspace target dir lives at the repo root, and watching
      // locked build artifacts makes chokidar throw EBUSY on Windows.
      ignored: ['**/target/**'],
    },
  },
}));
