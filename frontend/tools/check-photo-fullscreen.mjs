import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

const base = new URL(process.env.LAYOUT_BASE_URL || 'http://127.0.0.1:8080');
const output = process.env.LAYOUT_OUTPUT || '/tmp/home-hq-photo-fullscreen';
await mkdir(output, { recursive: true });
const browser = await chromium.launch({
  ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}),
});
try {
  for (const fallback of [false, true]) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, hasTouch: true });
    page.setDefaultTimeout(10000);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      window.screenLockEvents = [];
      Object.defineProperty(navigator, 'wakeLock', { configurable: true, value: {
        request: async () => {
          window.screenLockEvents.push('acquire');
          return { addEventListener() {}, release: async () => { window.screenLockEvents.push('release'); } };
        },
      } });
    });
    await page.goto(new URL('/?wall#photos', base).href, { waitUntil: 'networkidle' });
    await page.waitForFunction(() => App.current === 'photos' && DATA.photos.length > 0);
    await page.evaluate(fallback => {
      App.setNight({ from: '00:00', to: '00:00' });
      App.setIdle({ home: 5, rest: 5, peek: 5 });
      if (fallback) document.documentElement.requestFullscreen = () => Promise.reject(new Error('unsupported fullscreen'));
    }, fallback);
    const current = await page.locator('.ph-stage img:not(.blurfill)').first().getAttribute('src');
    await page.getByRole('button', { name: 'Full screen', exact: true }).click();
    await page.waitForFunction(() => App.photoFullscreen);
    if (!fallback) await page.waitForFunction(() => !!document.fullscreenElement);
    assert.equal(await page.locator('.ph-stage img:not(.blurfill)').first().getAttribute('src'), current);
    for (const selector of ['#rail', '.ph-top', '.ph-bottom', '.ph-zone', '.photos .cap', '#notes', '#banners', '#toasts']) {
      for (const el of await page.locator(selector).all()) assert.equal(await el.isVisible(), false, `${selector} is visible in full screen`);
    }
    assert.equal(await page.getByRole('button', { name: 'Exit full screen' }).isVisible(), true);
    if (fallback) await page.setViewportSize({ width: 800, height: 1100 });
    const stage = await page.locator('#stage').boundingBox();
    const photo = await page.locator('.ph-stage').boundingBox();
    assert.deepEqual(photo, stage, 'Photo does not fill the stage');
    const viewport = page.viewportSize();
    assert.ok(Math.abs(photo.width - viewport.width) < 1 && Math.abs(photo.height - viewport.height) < 1, 'Photo does not fill the viewport');
    await page.screenshot({ path: path.join(output, fallback ? 'fullscreen-fallback.png' : 'fullscreen-native.png') });
    // Real idle timer must not return Home or enter ambient/night/off.
    await page.waitForTimeout(6500);
    // Exercise scheduled sleep and presence-based sleep as well as the timer.
    await page.evaluate(() => {
      App.setNight({ from: '00:00', to: '23:59', offOn: true, offFrom: '00:00', offTo: '23:59' });
      DATA.homeOnline = true; DATA.awayOffMinutes = .000001;
      DATA.people.forEach(person => { person.tracked = true; person.status = 'Away'; person.home = false; });
      App.idleFor = () => 999;
      App.emit('minute', App.now()); App.emit('minute', App.now()); App.rest(); App.syncNightMode();
    });
    assert.equal(await page.evaluate(() => App.mode), 'day');
    assert.equal(await page.evaluate(() => App.current), 'photos');
    assert.ok((await page.evaluate(() => screenLockEvents)).includes('acquire'));
    await page.getByRole('button', { name: 'Exit full screen' }).click();
    await page.waitForFunction(() => !App.photoFullscreen && !document.fullscreenElement);
    assert.equal(await page.locator('#rail').isVisible(), true);
    assert.ok((await page.evaluate(() => screenLockEvents)).includes('release'));
    // Escape and navigation also clean up mode, layout and sleep suppression.
    await page.getByRole('button', { name: 'Full screen', exact: true }).click();
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => !App.photoFullscreen && !document.fullscreenElement);
    if (!fallback) {
      await page.getByRole('button', { name: 'Full screen', exact: true }).click();
      await page.waitForFunction(() => !!document.fullscreenElement);
      await page.evaluate(() => document.exitFullscreen());
      await page.waitForFunction(() => !App.photoFullscreen);
    }
    await page.getByRole('button', { name: 'Full screen', exact: true }).click();
    await page.evaluate(() => App.go('home'));
    await page.waitForFunction(() => !App.photoFullscreen && !document.fullscreenElement);
    await page.evaluate(() => App.rest());
    assert.notEqual(await page.evaluate(() => App.mode), 'day', 'Sleep did not resume after exiting full screen');
    assert.deepEqual(errors, []);
    await page.close();
    console.log(`PASS ${fallback ? 'fallback' : 'native'} full screen: same slide, hidden chrome, exit/Escape/navigation, wake lock, sleep suspended and restored`);
  }
} finally { await browser.close(); }
