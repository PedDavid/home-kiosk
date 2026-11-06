// The screenshot tests' harness: the built frontend (dist/) served at a fake origin with every
// /api call answered from fixtures/, at the PF1007L's 1280×800, with a pinned clock, seeded
// randomness, no motion and no network. Nothing here changes the app; it only controls what the
// page sees.
import { readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { test as base, expect } from '@playwright/test';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..');
const dist = path.join(root, 'dist');
const fixtures = path.join(here, 'fixtures');

export const ORIGIN = 'https://home-hq.test';
// Thursday 5 November 2026, 5:30 PM in Lisbon (UTC+0 in winter): daytime, before the night
// hours, inside the 4 PM bins window. The fixtures are rendered at this moment too; keep it in
// step with NOW in examples/screen_fixtures.rs and regenerate them with
// `cargo run --example screen_fixtures` after changing either.
export const NOW = '2026-11-05T17:30:00Z';
export const TIMEZONE = 'Europe/Lisbon';

// Ken Burns pans, toasts and sheets animate; a capture mid-way would differ run to run.
const NO_MOTION = '*, *::before, *::after { transition: none !important; animation: none !important; caret-color: transparent !important; }';

const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json',
  '.woff2': 'font/woff2', '.ttf': 'font/ttf', '.txt': 'text/plain',
};
const type = file => TYPES[path.extname(file)] || 'application/octet-stream';
export const fixture = name => JSON.parse(readFileSync(path.join(fixtures, name + '.json'), 'utf8'));

// The Google Fonts stylesheet index.html asks for, answered with the same families from the
// @fontsource packages (pinned in package.json), so text renders in the real typefaces without
// the network. A build that serves its fonts itself never makes this request.
function googleFontsCss() {
  const pkg = name => path.join(root, 'node_modules', name);
  const css = (name, file, family) => readFileSync(path.join(pkg(name), file), 'utf8')
    .replace(/font-family: '[^']+'/g, `font-family: '${family}'`)
    .replace(/url\(\.\/files\//g, `url(https://fonts.gstatic.com/fontsource/${name}/`);
  return [
    css('@fontsource-variable/archivo', 'wdth.css', 'Archivo'),
    css('@fontsource/instrument-serif', '400.css', 'Instrument Serif'),
    css('@fontsource/instrument-serif', '400-italic.css', 'Instrument Serif'),
  ].join('\n');
}
function fontFile(url) {
  const m = url.pathname.match(/^\/fontsource\/(.+)\/([^/]+\.woff2)$/);
  return m && path.join(root, 'node_modules', m[1], 'files', m[2]);
}

function staticFile(pathname) {
  // Same page routing as the Rust server (src/assets.rs): /m and /upload are pages.
  const page = { '/': 'index.html', '/m': 'm.html', '/upload': 'upload.html' }[pathname];
  const file = path.join(dist, page || decodeURIComponent(pathname.slice(1)));
  return file.startsWith(dist) && existsSync(file) && !file.endsWith(path.sep) ? file : null;
}

// Answers for every /api route the frontend calls. `api` overrides parts of the fixtures.
function apiResponse(req, url, api) {
  const p = url.pathname.slice(4);
  const json = body => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  if (req.method() !== 'GET') return json({ ok: true });
  const photo = p.match(/^\/photos\/([\w-]+)$/);
  if (photo) {
    const file = path.join(fixtures, 'photos', photo[1] + (url.searchParams.has('s') ? '-s' : '') + '.svg');
    return existsSync(file) ? { status: 200, contentType: 'image/svg+xml', body: readFileSync(file) } : { status: 404, body: '' };
  }
  if (p === '/ha/camera') {
    const file = path.join(fixtures, 'photos', url.searchParams.get('id') + '.svg');
    return existsSync(file) ? { status: 200, contentType: 'image/svg+xml', body: readFileSync(file) } : { status: 404, body: '' };
  }
  const name = { '/state': 'state', '/calendar': 'calendar', '/weather': 'weather', '/home': 'home', '/home/energy': 'energy', '/frames': 'frames', '/alerts': 'alerts' }[p];
  if (!name) return { status: 404, contentType: 'application/json', body: '{"error":"no fixture for ' + p + '"}' };
  const data = fixture(name);
  return json(api[name] ? api[name](data) : data);
}

// Seeded Math.random (mulberry32), so the slideshow's shuffle, Ken Burns offsets and polaroid
// tilt are the same every run. Runs before any page script.
function seedRandom(seed) {
  let a = seed >>> 0;
  Math.random = function () {
    a = (a + 0x6D2B79F5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const test = base.extend({
  // Per-test fixture overrides: { state: s => ({ ...s, photos: [...] }) }.
  api: [{}, { option: true }],
  // Requests that tried to leave the fake origin (the frame is LAN-only); asserted empty.
  external: async ({}, use) => { await use([]); },
  page: async ({ page, api, external }, use) => {
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.clock.setFixedTime(new Date(NOW));
    await page.addInitScript(seedRandom, 20261105);
    await page.addInitScript(css => {
      document.addEventListener('DOMContentLoaded', () => {
        const s = document.createElement('style');
        s.textContent = css;
        document.head.appendChild(s);
      });
    }, NO_MOTION);
    await page.routeWebSocket(/\/api\/alerts\/ws$/, ws => {
      ws.onMessage(m => { if (m === 'ping') ws.send('pong'); });
      ws.send(JSON.stringify(fixture('alerts')));
    });
    await page.route('**/*', route => {
      const req = route.request(), url = new URL(req.url());
      if (url.origin === ORIGIN) {
        if (url.pathname.startsWith('/api/')) return route.fulfill(apiResponse(req, url, api));
        const file = staticFile(url.pathname);
        return file ? route.fulfill({ status: 200, contentType: type(file), body: readFileSync(file) }) : route.fulfill({ status: 404, body: 'Not found' });
      }
      if (url.origin === 'https://fonts.googleapis.com') return route.fulfill({ status: 200, contentType: 'text/css', body: googleFontsCss() });
      if (url.origin === 'https://fonts.gstatic.com' && fontFile(url) && existsSync(fontFile(url))) {
        return route.fulfill({ status: 200, contentType: 'font/woff2', body: readFileSync(fontFile(url)) });
      }
      external.push(req.url());
      return route.abort();
    });
    await use(page);
    expect(errors, 'errors thrown in the page').toEqual([]);
    expect(external, 'requests outside the frame’s network').toEqual([]);
  },
});
export { expect };

// Everything that has to settle before a capture: the app has started and drawn its view, the
// fonts are in, and every image on screen has loaded (or failed).
export async function settle(page) {
  await page.waitForFunction(() => window.App && App.current && document.querySelector('#railClock b'));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(() => Array.from(document.images).every(i => i.complete));
  // Two frames, so layout and paint have caught up with the last change.
  await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
}

// Opens the wall view on a tab (#home, #calendar, ...), started and settled.
export async function open(page, hash = 'home', pathname = '/') {
  await page.goto(ORIGIN + pathname + (hash ? '#' + hash : ''));
  await settle(page);
}
