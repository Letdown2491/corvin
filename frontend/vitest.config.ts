import { defineConfig } from 'vitest/config'

// Standalone Vitest config — deliberately does NOT load the SvelteKit plugin.
// v1 tests cover pure TS logic in src/lib (no .svelte compilation, no DOM), so a
// plain node environment is all we need and avoids SvelteKit's $app/$env aliases.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.{test,spec}.ts'],
  },
})
