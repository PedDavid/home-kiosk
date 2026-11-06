// Run against the built Rust application. All external requests are blocked,
// matching the frame's LAN-only network. This checks geometry, not pixel hashes.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

const base = new URL(process.env.LAYOUT_BASE_URL || 'http://127.0.0.1:8080');
const output = process.env.LAYOUT_OUTPUT || '/tmp/home-hq-layout';
await mkdir(output, { recursive: true });
const browser = await chromium.launch({
  ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}),
});
try {
  for (const [width, height] of [[1280, 800], [1024, 600], [1078, 674], [1920, 1080]]) {
    const context = await browser.newContext({
      viewport: { width, height }, deviceScaleFactor: 1, hasTouch: true,
      timezoneId: 'Atlantic/Reykjavik',
    });
    const external = [];
    await context.route('**/*', route => {
      if (new URL(route.request().url()).origin === base.origin) return route.continue();
      external.push(route.request().url());
      return route.abort();
    });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(new URL('/?wall', base).href, { waitUntil: 'networkidle' });
    await page.waitForSelector('.h-time');
    await page.evaluate(() => {
      App.setNight({ from: '00:00', to: '00:00' });
      // A four-digit PM clock reproduces the original overlap with fallback fonts.
      App.now = () => new Date('2026-11-05T22:48:00Z');
      App.emit('data');
    });
    await page.evaluate(() => document.fonts.ready);
    const failures = await page.evaluate(() => {
      const failures = [];
      const rect = selector => document.querySelector(selector).getBoundingClientRect();
      const clock = rect('.h-clock'), ampm = rect('.h-ampm'), icon = rect('.h-wx-icon');
      if (ampm.right > clock.right + 1 || ampm.right > icon.left) failures.push('Clock overlaps weather');
      const stage = rect('#stage');
      if (stage.left < -1 || stage.right > innerWidth + 1 || stage.bottom > innerHeight + 1) failures.push('Stage outside viewport');
      for (const hour of document.querySelectorAll('.h-hr')) {
        const label = hour.querySelector('.h-hr-l').getBoundingClientRect();
        const icon = hour.querySelector('.h-hr-i').getBoundingClientRect();
        if (label.bottom > icon.top) failures.push('Weather hour overlaps icon');
      }
      for (const selector of ['.h-hr-l', '.h-hr-i', '.h-hr > b']) {
        const tops = [...document.querySelectorAll(selector)].map(el => el.getBoundingClientRect().top);
        if (Math.max(...tops) - Math.min(...tops) > 0.1) failures.push(`${selector} is not aligned across forecast hours`);
      }
      if (innerWidth === 1280 && innerHeight === 800) {
        for (const selector of ['.h-presence', '.h-side', '.h-shop', '.h-status']) {
          const r = rect(selector);
          if (Math.abs(r.left - Math.round(r.left)) > 0.01 || Math.abs(r.right - Math.round(r.right)) > 0.01) failures.push(`${selector} has fractional column edges`);
        }
      }
      // Check nested cards as well as the outer grid: overflow:hidden can hide
      // broken layout while leaving the document's scrollWidth unchanged.
      for (const element of document.querySelectorAll('.home > *, .h-side > *, .h-duo > *')) {
        const r = element.getBoundingClientRect();
        if (r.left < stage.left - 1 || r.right > stage.right + 1) failures.push(`${element.className} outside frame`);
      }
      const faces = [...document.fonts];
      if (faces.length !== 3 || faces.some(face => face.status !== 'loaded')) failures.push('Offline fonts not loaded');
      return failures;
    });
    const filename = path.join(output, `frame-${width}x${height}.png`);
    await page.screenshot({ path: filename });
    assert.deepEqual(errors, [], 'Browser errors');
    assert.deepEqual(external.filter(url => /fonts\.googleapis\.com|fonts\.gstatic\.com/.test(url)), [], 'External font dependency');
    assert.deepEqual(failures, [], `${width}x${height} layout: ${filename}`);
    console.log(`PASS ${width}x${height}: offline fonts, clock/weather spacing, card bounds and native column alignment (${filename})`);
    await context.close();
  }
} finally {
  await browser.close();
}
