// Screenshots of the wall view at the PF1007L's 1280×800, compared with tests/screens/screenshots/.
// Each test also runs the layout check: nothing may spill out of the 1280×800 frame.
import { test, expect, open, settle, fixture } from './kiosk.mjs';
import { layoutProblems } from './layout.mjs';

// The demo library has landscape photos (demo0, "on this day") and portrait ones (demo2).
const only = id => ({ state: s => ({ ...s, photos: s.photos.filter(p => p.id === id) }) });

async function fits(page) {
  expect(await page.evaluate(layoutProblems), 'layout problems at 1280×800').toEqual([]);
}

test.describe('tabs', () => {
  test('home', async ({ page }) => {
    await open(page, 'home');
    await expect(page).toHaveScreenshot('home.png');
  });

  test('home layout', async ({ page }) => {
    await open(page, 'home');
    await fits(page);
  });

  for (const tab of ['calendar', 'shopping', 'control', 'media']) {
    test(tab, async ({ page }) => {
      await open(page, tab);
      await expect(page).toHaveScreenshot(`${tab}.png`);
      await fits(page);
    });
  }
});

test.describe('photos', () => {
  test.describe('landscape', () => {
    test.use({ api: only('demo0') });
    test('photos-landscape', async ({ page }) => {
      await open(page, 'photos');
      await expect(page.locator('#view-photos .slide--cover')).toBeVisible();
      await expect(page).toHaveScreenshot('photos-landscape.png');
      await fits(page);
    });
  });

  test.describe('portrait', () => {
    test.use({ api: only('demo2') });
    test('photos-portrait', async ({ page }) => {
      await open(page, 'photos');
      // Auto shows a lone portrait as a polaroid; Cover fits it over a blurred fill.
      await page.locator('#view-photos [data-layout=cover]').click();
      await expect(page.locator('#view-photos .slide--fit')).toBeVisible();
      await settle(page);
      await expect(page).toHaveScreenshot('photos-portrait.png');
      await fits(page);
    });

    test('photos-portrait-polaroid', async ({ page }) => {
      await open(page, 'photos');
      await expect(page.locator('#view-photos .slide--polaroid')).toBeVisible();
      await expect(page).toHaveScreenshot('photos-portrait-polaroid.png');
      await fits(page);
    });
  });

  test('photos-idle', async ({ page }) => {
    // The idle photo screen (ambient mode), as the idle timer leaves it: App.rest() is what
    // runs when the rest timeout runs out during the day.
    await open(page, 'photos');
    await page.evaluate(() => App.rest());
    await expect(page.locator('#stage')).toHaveAttribute('data-mode', 'ambient');
    await settle(page);
    await expect(page).toHaveScreenshot('photos-idle.png');
    await fits(page);
  });
});

// Removing a favorite (#6): the heart opens a sheet, and only holding Remove for a second acts.
// demo0 is a favorite in the fixtures; demo1 isn't.
test.describe('remove favorite', () => {
  const favPatches = page => {
    const sent = [];
    page.on('request', r => { if (r.method() === 'PATCH' && /\/api\/photos\//.test(r.url())) sent.push(r.postDataJSON()); });
    return sent;
  };
  const heart = page => page.locator('#view-photos [data-fav]');
  const sheet = page => page.locator('#sheetLayer.is-open .sheet--unfav');
  async function holdRemove(page, ms) {
    const b = await page.locator('[data-remove]').boundingBox();
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
    await page.mouse.down();
    await page.waitForTimeout(ms);
    await page.mouse.up();
  }

  test.describe('on a favorite', () => {
    test.use({ api: only('demo0') });

    test('photos-unfav-sheet', async ({ page }) => {
      await open(page, 'photos');
      await heart(page).click();
      await expect(sheet(page)).toBeVisible();
      await expect(page.locator('[data-cancel]')).toBeFocused();
      await settle(page);
      await expect(page).toHaveScreenshot('photos-unfav-sheet.png');
      await fits(page);
    });

    test('cancel, close, scrim and a short tap keep it', async ({ page }) => {
      const sent = favPatches(page);
      await open(page, 'photos');
      for (const dismiss of [
        () => page.locator('[data-cancel]').click(),
        () => page.locator('#sheetLayer .sheet-x').click(),
        () => page.mouse.click(40, 40),
      ]) {
        await heart(page).click();
        await expect(sheet(page)).toBeVisible();
        await dismiss();
        await expect(sheet(page)).toHaveCount(0);
      }
      await heart(page).click();
      await holdRemove(page, 300);
      await expect(page.locator('.ph-unfav-hint')).toBeVisible();
      await expect(sheet(page)).toBeVisible();
      await expect(heart(page)).toHaveClass(/is-on/);
      expect(sent).toEqual([]);
    });

    test('holding Remove unfavorites it', async ({ page }) => {
      const sent = favPatches(page);
      await open(page, 'photos');
      await heart(page).click();
      await holdRemove(page, 1200);
      await expect(sheet(page)).toHaveCount(0);
      await expect(heart(page)).not.toHaveClass(/is-on/);
      await expect(page.locator('.toast')).toContainText('Removed from favorites');
      expect(sent).toEqual([{ fav: false }]);
    });
  });

  test.describe('with Immich connected', () => {
    test.use({ api: { state: s => ({ ...s, photos: s.photos.filter(p => p.id === 'demo0'), photoAuth: { status: 'connected' } }) } });
    test('says it is removed in Immich too', async ({ page }) => {
      await open(page, 'photos');
      await heart(page).click();
      await expect(sheet(page)).toContainText('also removed from your favorites in Immich');
    });
  });

  test.describe('on a non-favorite', () => {
    test.use({ api: only('demo1') });
    test('one tap adds it', async ({ page }) => {
      const sent = favPatches(page);
      await open(page, 'photos');
      await heart(page).click();
      await expect(heart(page)).toHaveClass(/is-on/);
      await expect(page.locator('#sheetLayer.is-open')).toHaveCount(0);
      expect(sent).toEqual([{ fav: true }]);
    });
  });
});

// Share photo (#21): an icon-only Share button beside the full-screen button opens a sheet with a
// QR code for the photo in the Immich phone app (shareAppUrl) or, switched to Web, in a browser
// (shareUrl). The backend only sets both on Immich photos when HQ_IMMICH_PUBLIC_URL is set; the
// fixtures are demo photos, so these tests give demo0 an asset ID and both links.
test.describe('share photo', () => {
  const ASSET = '9b2d8a4e-3f1c-4e7a-b0d5-6c8f1e2a3b4c';
  const WEB = 'https://photos.example.com/photos/' + ASSET;
  const APP = 'immich://asset?id=' + ASSET;
  const immich = extra => ({ state: s => ({ ...s, photos: s.photos.filter(p => p.id === 'demo0').map(p => ({ ...p, immichId: ASSET, ...extra })) }) });
  const shareBtn = page => page.locator('#view-photos [data-share]');
  const sheet = page => page.locator('#sheetLayer.is-open .sheet--share');
  // The QR on screen is the one App.share draws for `url` (the same 400 px rendering).
  const qrIs = (page, url) => page.evaluate(u => {
    const want = document.createElement('div');
    want.innerHTML = App.share.qrSvg(u, 400);
    return document.querySelector('.share-qr').innerHTML === want.innerHTML;
  }, url);

  test.describe('on an Immich photo', () => {
    test.use({ api: immich({ shareUrl: WEB, shareAppUrl: APP }) });

    test('share-sheet', async ({ page }) => {
      await open(page, 'photos');
      await expect(shareBtn(page)).toBeVisible();
      await shareBtn(page).click();
      await expect(sheet(page)).toBeVisible();
      await expect(page.locator('[data-qr=app]')).toHaveClass(/is-on/);
      expect(await qrIs(page, APP), 'QR opens the Immich app by default').toBe(true);
      // Outside the kiosk app the web link is a real link, opened in a new tab.
      await expect(page.locator('a.share-link')).toHaveAttribute('href', WEB);
      await expect(page.locator('a.share-link')).toHaveAttribute('target', '_blank');
      await settle(page);
      await expect(page).toHaveScreenshot('share-sheet.png');
      await fits(page);
    });

    test('share-sheet-web', async ({ page }) => {
      await open(page, 'photos');
      await shareBtn(page).click();
      await page.locator('[data-qr=web]').click();
      await expect(page.locator('[data-qr=web]')).toHaveClass(/is-on/);
      expect(await qrIs(page, WEB), 'QR encodes the web link').toBe(true);
      await settle(page);
      await expect(page).toHaveScreenshot('share-sheet-web.png');
      await fits(page);
      // The choice is remembered on this screen.
      await page.locator('#sheetLayer .sheet-x').click();
      await shareBtn(page).click();
      await expect(page.locator('[data-qr=web]')).toHaveClass(/is-on/);
    });

    test('share-sheet-fullscreen', async ({ page }) => {
      await open(page, 'photos');
      await page.locator('#view-photos [data-photo-fullscreen]').click();
      await expect(page.locator('#stage')).toHaveClass(/is-photo-fullscreen/);
      await expect(shareBtn(page)).toBeVisible();
      await shareBtn(page).click();
      await expect(sheet(page)).toBeVisible();
      await settle(page);
      await expect(page).toHaveScreenshot('share-sheet-fullscreen.png');
      await fits(page);
    });
  });

  test.describe('on the frame (kiosk app)', () => {
    test.use({ api: immich({ shareUrl: WEB, shareAppUrl: APP }) });
    test('the web link is plain text', async ({ page }) => {
      // Following a link in the kiosk WebView would take the frame away from the dashboard.
      await page.addInitScript(() => { window.HomeHQKiosk = { info: () => '{}', setScreenOff() {} }; });
      await open(page, 'photos');
      await shareBtn(page).click();
      await expect(page.locator('span.share-link')).toHaveText('photos.example.com/photos/' + ASSET);
      await expect(page.locator('a.share-link')).toHaveCount(0);
    });
  });

  test.describe('without HQ_IMMICH_PUBLIC_URL', () => {
    test.use({ api: immich({}) });
    test('Share is hidden', async ({ page }) => {
      await open(page, 'photos');
      await expect(shareBtn(page)).toBeHidden();
    });
  });

  test.describe('on a demo photo', () => {
    test.use({ api: only('demo0') });
    test('Share is hidden', async ({ page }) => {
      await open(page, 'photos');
      await expect(shareBtn(page)).toBeHidden();
    });
  });
});

test('night', async ({ page }) => {
  await open(page, 'home');
  await page.evaluate(() => App.setMode('night', true));
  await settle(page);
  await expect(page).toHaveScreenshot('night.png');
  await fits(page);
});

test('note-sheet', async ({ page }) => {
  await open(page, 'calendar');
  await page.locator('#rail [data-note-new]').click();
  await expect(page.locator('#sheetLayer.is-open .sheet')).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot('note-sheet.png');
  await fits(page);
});

// The weather card and hourly strip on a real Open-Meteo forecast: the recorded snapshot
// (tests/fixtures/weather/open-meteo-snapshot.json) as the backend serves it. The clock is noon on
// the snapshot's day, so the strip starts mid-day whenever the snapshot was recorded.
test.describe('weather', () => {
  const forecast = fixture('weather-open-meteo');
  test.use({ api: { weather: () => forecast }, now: forecast.day + 'T12:00:00Z' });
  test('weather-open-meteo', async ({ page }) => {
    await open(page, 'home');
    await expect(page.locator('.h-wx-hours')).toBeVisible();
    await expect(page.locator('.h-weather')).toHaveScreenshot('weather-open-meteo.png');
  });
});
