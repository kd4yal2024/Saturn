import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    emptyOutDir: true,
    outDir: 'dist',
    sourcemap: true,
    lib: {
      entry: 'src/remote-next-entry.ts',
      name: 'SaturnRemoteNextBundle',
      formats: ['iife'],
      fileName: () => 'saturn-remote-next.js',
    },
  },
});
