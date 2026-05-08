import { defineConfig } from 'vite';

export default defineConfig({
  clearScreen: false,
  server: { port: 1431, strictPort: true },
  envPrefix: ['VITE_', 'TAURI_'],
});
