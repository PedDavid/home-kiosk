// Browser regression test for #7: tapping the photo on Home opens that same photo in Photos,
// both the first time and when Photos was already open before, and the slideshow carries on
// from there.
//
// Needs a build (npx vite build) and Playwright with Chromium: `npm run test:e2e`.
// The API is mocked in the page, so no backend is needed.
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright'); // CommonJS resolution, so NODE_PATH / global installs work too

const DIST = fileURLToPath(new URL('../dist/', import.meta.url));
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.png': 'image/png', '.webmanifest': 'application/manifest+json' };

// A 1x1 PNG for every photo; the test checks which photo id is on screen, not the pixels.
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==', 'base64');
// Landscape and portrait, a favourite, none on today's date (keeps the queue order random).
const PHOTOS = Array.from({ length: 8 }, (_, i) => ({
  id: 'p' + (i + 1), date: '2020-0' + ((i % 6) + 1) + '-1' + i, o: i % 3 === 2 ? 'p' : 'l',
  place: 'Place ' + (i + 1), from: 'a', tag: 't' + (i % 2), fav: i === 0,
}));
const API = {
  state: {
    people: [{ id: 'a', name: 'Alex', color: '#335577', rgb: '51 85 119', ink: '#FFFFFF' }],
    stores: [{ id: 'grocer', name: 'Groceries' }], catOrder: { all: ['Other'], grocer: ['Other'] },
    catalog: [], usuals: [], groceries: [], photos: PHOTOS,
    hasHome: false, scenes: [], cameras: [], bins: null, notes: [], awayOffMinutes: 0,
  },
  calendar: { events: [], errors: [] },
  weather: { none: true },
  home: { none: true },
  alerts: {},
};

function serve() {
  const server = createServer(async (req, res) => {
    let path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (path === '/') path = '/index.html';
    try {
      const file = normalize(join(DIST, path));
      if (!file.startsWith(DIST)) throw new Error('outside dist');
      const body = await readFile(file);
      res.writeHead(200, { 'content-type': TYPES[extname(file)] || 'application/octet-stream' });
      res.end(body);
    } catch {
      res.writeHead(404); res.end();
    }
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok(server)));
}

// Id of the photo a slideshow stage shows now (its newest slide, main image).
const showing = (page, stage) => page.evaluate((stage) => {
  const img = document.querySelector(stage + ' > .slide:last-child img:not(.blurfill)');
  const m = img && img.getAttribute('src').match(/\/api\/photos\/([^?]+)/);
  return m ? m[1] : null;
}, stage);
const photosShowing = (page) => showing(page, '#view-photos .ph-stage');
const homeShowing = (page) => showing(page, '#view-home .h-photo-stage');

async function main() {
  const server = await serve();
  const base = 'http://127.0.0.1:' + server.address().port;
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, screen: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.route('**/api/**', (route) => {
    const path = new URL(route.request().url()).pathname.replace(/^\/api\//, '');
    if (path.startsWith('photos/')) return route.fulfill({ contentType: 'image/png', body: PNG });
    const data = API[path];
    return data ? route.fulfill({ json: data }) : route.fulfill({ status: 404, json: { error: 'not mocked' } });
  });
  // Fake timers, so the home tile's 12 s slideshow can be moved on without waiting.
  await page.clock.install();

  try {
    await page.goto(base + '/?wall#home');
    await page.waitForSelector('#view-home .h-photo-stage > .slide img');
    // Let the app pre-build the hidden views (Photos included: its own slideshow already picked a
    // photo, which is how the bug showed up).
    await page.clock.runFor(5000);

    // 1. First tap: Photos opens on the photo Home was showing.
    let id = await homeShowing(page);
    await page.click('#view-home .h-photo');
    await page.waitForSelector('#view-photos.is-active');
    assert.equal(await photosShowing(page), id, 'Photos should open on the photo tapped on Home');

    // 2. Normal navigation carries on: next shows another photo, previous comes back to it.
    await page.click('#view-photos .ph-bottom [data-next]');
    const after = await photosShowing(page);
    assert.ok(after, 'next should show a photo');
    await page.click('#view-photos .ph-bottom [data-prev]');
    assert.equal(await photosShowing(page), id, 'previous should return to the tapped photo');
    await page.click('#view-photos .ph-bottom [data-next]');

    // 3. Photos was already open: back to Home, let the tile move to a photo Photos isn't on, tap again.
    await page.click('#rail [data-go="home"]');
    await page.waitForSelector('#view-home.is-active');
    const inPhotos = await photosShowing(page);
    for (let i = 0; i < 20 && (await homeShowing(page)) === inPhotos; i++) await page.clock.runFor(12500);
    id = await homeShowing(page);
    assert.notEqual(id, inPhotos, 'test setup: Home should show a different photo than Photos');
    await page.click('#view-home .h-photo');
    await page.waitForSelector('#view-photos.is-active');
    assert.equal(await photosShowing(page), id, 'reopened Photos should show the photo tapped on Home');

    // 4. Opening Photos from the rail (no photo passed) keeps its own slideshow position.
    await page.click('#rail [data-go="home"]');
    await page.click('#rail [data-go="photos"]');
    assert.equal(await photosShowing(page), id, 'Photos from the rail should stay where it was');

    // 5. The slideshow timer still advances afterwards.
    await page.clock.runFor(15500);
    assert.notEqual(await photosShowing(page), null);

    assert.deepEqual(errors, [], 'no page errors');
    console.log('ok - home photo opens the same photo in Photos');
  } finally {
    await browser.close();
    server.close();
  }
}

main().catch((e) => { console.error('not ok -', e.message); process.exit(1); });
