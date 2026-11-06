// Compare actual rendered pixels: descendant filter:none cannot undo an ancestor's filter.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

const base = new URL(process.env.LAYOUT_BASE_URL || 'http://127.0.0.1:8080');
const output = process.env.LAYOUT_OUTPUT || '/tmp/home-hq-photo-colours';
await mkdir(output, { recursive: true });
const browser = await chromium.launch({
  ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}),
});
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.route('**/api/photos/**', route => route.fulfill({
    contentType: 'image/svg+xml',
    body: '<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1280"><path fill="#e94c36" d="M0 0h640v1280H0z"/><path fill="#39a86b" d="M640 0h640v1280H640z"/><path fill="#398ad7" d="M1280 0h640v1280H1280z"/></svg>',
  }));
  await page.goto(new URL('/?wall', base).href, { waitUntil: 'networkidle' });
  await page.evaluate(() => {
    App.setNight({ from: '00:00', to: '00:00' });
    App.setIdle({ home: 3600, rest: 3600 });
  });
  for (const [view, selector] of [['home', '.h-photo-stage'], ['photos', '.ph-stage'], ['ambient', '.amb-stage']]) {
    await page.evaluate(view => {
      App.setMode('day');
      App.go(view === 'ambient' ? 'photos' : view);
      if (view === 'ambient') App.setMode('ambient', true);
    }, view);
    await page.locator(selector + ' img:not(.blurfill)').first().waitFor();
    await page.waitForFunction(selector => {
      const im = document.querySelector(selector + ' img:not(.blurfill)');
      return im && im.complete && im.naturalWidth > 0;
    }, selector);
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(100);
    const box = await page.locator(selector).boundingBox();
    // Sample three colour patches near the top, away from text, navigation and captions.
    const clips = [.15, .5, .85].map(f => ({ x: Math.floor(box.x + box.width * f), y: Math.floor(box.y + box.height * .25), width: 12, height: 12 }));
    const day = await Promise.all(clips.map(clip => page.screenshot({ clip })));
    await page.evaluate(() => {
      document.querySelector('#stage').dataset.night = '1';
      // Keep the scheduled clock from changing the test condition during capture.
      App.applyNight = () => {};
    });
    const night = await Promise.all(clips.map(clip => page.screenshot({ clip })));
    for (let i = 0; i < day.length; i++) assert.deepEqual(night[i], day[i], `${view}: photo colours changed at night (patch ${i})`);
    assert.notEqual(await page.locator('#rail').evaluate(el => getComputedStyle(el).filter), 'none', 'Night UI lost its dimming');
    await page.screenshot({ path: path.join(output, `${view}-night.png`) });
    await page.evaluate(() => { document.querySelector('#stage').dataset.night = '0'; });
    console.log(`PASS ${view}: photo pixels unchanged at night; UI night filter retained`);
  }
} finally {
  await browser.close();
}
