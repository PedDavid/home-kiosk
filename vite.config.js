import { defineConfig } from 'vite';
import pkg from './package.json' with { type: 'json' };

// Changes with every build; frames compare it with /build.json and reload after a deploy.
const BUILD_ID = Date.now().toString(36);

// The wall tablet runs Android WebView 106 — keep JS output within Chromium 106.
// CSS targets chrome61 on purpose: the tablet's WebView rejects 4/8-digit hex colours (#0000,
// #ffffffbf) even though Chrome 106 should support them, and a newer target lets the minifier
// rewrite transparent/rgba() into that form. Verified on-device with CSS.supports().
export default defineConfig({
  // Shown under Status in the display controls. Bump "version" in package.json for each release.
  define: { __APP_VERSION__: JSON.stringify(pkg.version.replace(/\.0$/, '')), __BUILD_ID__: JSON.stringify(BUILD_ID) },  // 1.2.0 shows as 1.2
  plugins: [{
    name: 'build-json',
    generateBundle() { this.emitFile({ type: 'asset', fileName: 'build.json', source: JSON.stringify({ build: BUILD_ID, version: pkg.version }) }); },
  }],
  build: {
    target: 'chrome106',
    cssTarget: 'chrome61',
    rollupOptions: { input: { main: 'index.html', upload: 'upload.html', m: 'm.html' } },
  },
  // `npm run dev` serves the UI; /api goes to `npm run dev:api` (wrangler dev, local D1 + R2).
  server: { host: true, proxy: { '/api': 'http://localhost:8787' } },
});
