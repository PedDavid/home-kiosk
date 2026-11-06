// Screenshots of the wall view at the PF1007L's 1280×800, compared with tests/screens/screenshots/.
// Each test also runs the layout check: nothing may spill out of the 1280×800 frame.
import { test, expect, open, settle } from './kiosk.mjs';
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
